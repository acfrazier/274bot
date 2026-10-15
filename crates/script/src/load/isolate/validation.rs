//! One process validation coordinator. Its bounded queue and single seat
//! prevent a refresh from starting an isolate per card or per window.

use super::super::library::PreparedCard;
use super::{LoadIsolate, LoadIsolateError, LoadShape, Ready, JOIN_TIMEOUT, VALIDATION_TIMEOUT};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

const QUEUE_CAP: usize = 16;
const SOURCE_CAP: usize = 4 * 1024 * 1024;
const SIBLING_CAP: usize = 32;
const INPUT_CAP: usize = 8 * 1024 * 1024;
const BATCH_CAP: usize = 256;
const BATCH_BYTES_CAP: usize = 16 * 1024 * 1024;

pub(super) type Batch = Vec<(PreparedCard, Result<(), String>)>;
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error(transparent)]
    Platform(#[from] LoadIsolateError),
    #[error("validation batch exceeds the card limit")]
    CardLimit,
    #[error("validation batch exceeds the byte limit")]
    ByteLimit,
    #[error("validation coordinator: {0}")]
    Coordinator(String),
    #[error("validation coordinator unavailable")]
    Unavailable,
}

/// Owning reload receipt. A full bounded queue leaves this one transaction
/// here for a later non-blocking poll. Dropping it cancels pending or queued
/// work.
pub struct ValidationBatch {
    receiver: Receiver<Batch>,
    cancelled: Arc<AtomicBool>,
    pending: RefCell<Option<Job>>,
}

impl ValidationBatch {
    pub fn try_recv(&self) -> Result<Batch, TryRecvError> {
        let seat = SEAT.as_ref().map_err(|_| TryRecvError::Disconnected)?;
        try_admit_pending(seat, &self.pending)?;
        self.receiver.try_recv()
    }
}

fn try_admit_pending(
    seat: &SyncSender<Job>,
    pending: &RefCell<Option<Job>>,
) -> Result<(), TryRecvError> {
    let Some(job) = pending.borrow_mut().take() else {
        return Ok(());
    };
    match seat.try_send(job) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(job)) => {
            pending.replace(Some(job));
            Err(TryRecvError::Empty)
        }
        Err(TrySendError::Disconnected(_)) => Err(TryRecvError::Disconnected),
    }
}

impl Drop for ValidationBatch {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

enum Job {
    Single {
        js: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        reply: mpsc::Sender<Result<(), String>>,
    },
    Batch {
        cards: Vec<PreparedCard>,
        reply: mpsc::Sender<Batch>,
        cancelled: Arc<AtomicBool>,
    },
}

static EMPTY_BANKS: LazyLock<std::sync::Arc<api::named_banks::NamedBankFacts>> =
    LazyLock::new(|| std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()));

static SEAT: LazyLock<Result<SyncSender<Job>, String>> = LazyLock::new(|| {
    let (tx, rx) = mpsc::sync_channel(QUEUE_CAP);
    std::thread::Builder::new()
        .name("js-validation-coordinator".into())
        .spawn(move || run(rx))
        .map_err(|error| error.to_string())?;
    Ok(tx)
});

pub(super) fn validate(
    js: &str,
    shape: LoadShape,
    siblings: &[(String, String)],
) -> Result<(), String> {
    super::super::platform::require_v8_platform().map_err(|error| error.to_string())?;
    if js.len() > SOURCE_CAP || siblings.len() > SIBLING_CAP {
        return Err("validation input exceeds the seat limit".into());
    }
    let total = siblings.iter().try_fold(js.len(), |bytes, (name, source)| {
        bytes.checked_add(name.len())?.checked_add(source.len())
    });
    if total.is_none_or(|bytes| bytes > INPUT_CAP) {
        return Err("validation input exceeds the seat byte limit".into());
    }
    let seat = SEAT
        .as_ref()
        .map_err(|error| format!("validation coordinator: {error}"))?;
    let (tx, rx) = mpsc::channel();
    let job = Job::Single {
        js: js.to_owned(),
        shape,
        siblings: siblings.to_vec(),
        reply: tx,
    };
    match seat.try_send(job) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => return Err("validation queue is full".into()),
        Err(TrySendError::Disconnected(_)) => {
            return Err("validation coordinator unavailable".into())
        }
    }
    rx.recv_timeout(VALIDATION_TIMEOUT + JOIN_TIMEOUT + Duration::from_secs(2))
        .unwrap_or_else(|_| Err("validation coordinator did not settle within its deadline".into()))
}

/// Submit one complete reload validation transaction without a per-reload
/// thread. A full worker queue leaves the batch on its receipt for a later
/// non-blocking poll; results return in input order.
pub(super) fn validate_batch(cards: Vec<PreparedCard>) -> Result<ValidationBatch, ValidationError> {
    super::super::platform::require_v8_platform()?;
    if cards.len() > BATCH_CAP {
        return Err(ValidationError::CardLimit);
    }
    let bytes = cards.iter().try_fold(0usize, |sum, card| {
        let total = card
            .siblings
            .iter()
            .try_fold(card.card.js.len(), |n, (name, js)| {
                n.checked_add(name.len())?.checked_add(js.len())
            })?;
        if card.card.js.len() > SOURCE_CAP || card.siblings.len() > SIBLING_CAP || total > INPUT_CAP
        {
            return None;
        }
        sum.checked_add(total)
    });
    if bytes.is_none_or(|bytes| bytes > BATCH_BYTES_CAP) {
        return Err(ValidationError::ByteLimit);
    }
    let seat = SEAT
        .as_ref()
        .map_err(|error| ValidationError::Coordinator(error.clone()))?;
    let (reply, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let pending = match seat.try_send(Job::Batch {
        cards,
        reply,
        cancelled: Arc::clone(&cancelled),
    }) {
        Ok(()) => None,
        Err(TrySendError::Full(job)) => Some(job),
        Err(TrySendError::Disconnected(_)) => return Err(ValidationError::Unavailable),
    };
    Ok(ValidationBatch {
        receiver,
        cancelled,
        pending: RefCell::new(pending),
    })
}

fn run(rx: Receiver<Job>) {
    // Validation admission stays occupied through a credited quarantine.
    let mut retiring: Option<Receiver<Vec<String>>> = None;
    for job in rx {
        if matches!(&job, Job::Batch { cancelled, .. } if cancelled.load(Ordering::Acquire)) {
            continue;
        }
        if retiring_pending(&mut retiring) {
            refuse(job, "validation isolate is still retiring");
            continue;
        }
        match job {
            Job::Single {
                js,
                shape,
                siblings,
                reply,
            } => {
                let result = validate_one(js, shape, siblings, &mut retiring);
                let _ = reply.send(result);
            }
            Job::Batch {
                cards,
                reply,
                cancelled,
            } => {
                let mut results = Vec::with_capacity(cards.len());
                for card in cards {
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let outcome = if retiring_pending(&mut retiring) {
                        Err("validation isolate is still retiring".into())
                    } else {
                        validate_one(
                            card.card.js.clone(),
                            card.card.shape,
                            card.siblings.clone(),
                            &mut retiring,
                        )
                    }
                    .map_err(|error| format!("prepare {}: {error}", card.card.name));
                    results.push((card, outcome));
                }
                let _ = reply.send(results);
            }
        }
    }
}

fn retiring_pending(retiring: &mut Option<Receiver<Vec<String>>>) -> bool {
    match retiring.as_ref().map(|done| done.try_recv()) {
        Some(Ok(_)) => {
            *retiring = None;
            false
        }
        // The supervisor sends a receipt only after physical join. A closed
        // receipt channel is not evidence that the isolate joined.
        Some(Err(TryRecvError::Empty | TryRecvError::Disconnected)) => true,
        None => false,
    }
}

fn refuse(job: Job, reason: &str) {
    match job {
        Job::Single { reply, .. } => {
            let _ = reply.send(Err(reason.into()));
        }
        Job::Batch { cards, reply, .. } => {
            let rows = cards
                .into_iter()
                .map(|card| {
                    let error = format!("prepare {}: {reason}", card.card.name);
                    (card, Err(error))
                })
                .collect();
            let _ = reply.send(rows);
        }
    }
}

fn validate_one(
    js: String,
    shape: LoadShape,
    siblings: Vec<(String, String)>,
    retiring: &mut Option<Receiver<Vec<String>>>,
) -> Result<(), String> {
    let isolate = LoadIsolate::spawn_inner(
        js,
        shape,
        siblings,
        None,
        std::sync::Arc::clone(&EMPTY_BANKS),
    )
    .map_err(|error| error.to_string())?;
    let result = match isolate.resolve_setup(Some(Instant::now() + VALIDATION_TIMEOUT)) {
        Ready::Ready => Ok(()),
        Ready::Failed(error) => Err(error),
        Ready::Pending => Err("validation setup did not settle".into()),
    };
    let (done_tx, done_rx) = mpsc::channel();
    isolate.retire(false, true, done_tx);
    match done_rx.recv_timeout(JOIN_TIMEOUT + Duration::from_millis(250)) {
        Ok(_) => result,
        Err(_) => {
            // Setup readiness is the validation result. Keep the seat
            // quarantined until join, but do not turn a ready card into a
            // validation failure merely because teardown is still pending.
            *retiring = Some(done_rx);
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch_job() -> Job {
        let (reply, _) = mpsc::channel();
        Job::Batch {
            cards: Vec::new(),
            reply,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn pending_batch_admission_retries_without_growing_the_queue() {
        let (seat, rx) = mpsc::sync_channel(1);
        assert!(matches!(seat.try_send(batch_job()), Ok(())));

        let pending = RefCell::new(Some(batch_job()));
        assert_eq!(try_admit_pending(&seat, &pending), Err(TryRecvError::Empty));

        drop(rx.try_recv().unwrap());
        try_admit_pending(&seat, &pending).unwrap();
        assert!(pending.borrow_mut().take().is_none());
        assert!(matches!(rx.try_recv(), Ok(Job::Batch { .. })));
    }

    #[test]
    fn validation_permit_is_retained_until_join_receipt() {
        let (joined, done) = mpsc::channel();
        let mut retiring = Some(done);
        assert!(retiring_pending(&mut retiring));
        assert!(retiring.is_some());

        joined.send(Vec::new()).unwrap();
        assert!(!retiring_pending(&mut retiring));
        assert!(retiring.is_none());

        let (abandoned, done) = mpsc::channel();
        let mut retiring = Some(done);
        drop(abandoned);
        assert!(retiring_pending(&mut retiring));
        assert!(retiring.is_some());
    }
}
