//! One process validation coordinator. Its bounded queue and single seat
//! prevent a refresh from starting an isolate per card or per window.

use super::super::library::PreparedCard;
use super::{LoadIsolate, LoadIsolateError, LoadShape, Ready, JOIN_TIMEOUT, VALIDATION_TIMEOUT};
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
    #[error("validation queue is full")]
    QueueFull,
    #[error("validation coordinator unavailable")]
    Unavailable,
}

/// Owning reload receipt. Dropping it cancels queued/remaining card work.
pub struct ValidationBatch {
    receiver: Receiver<Batch>,
    cancelled: Arc<AtomicBool>,
}

impl ValidationBatch {
    pub fn try_recv(&self) -> Result<Batch, TryRecvError> {
        self.receiver.try_recv()
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
/// thread. The coordinator returns the original prepared cards in order.
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
    match seat.try_send(Job::Batch {
        cards,
        reply,
        cancelled: Arc::clone(&cancelled),
    }) {
        Ok(()) => Ok(ValidationBatch {
            receiver,
            cancelled,
        }),
        Err(TrySendError::Full(_)) => Err(ValidationError::QueueFull),
        Err(TrySendError::Disconnected(_)) => Err(ValidationError::Unavailable),
    }
}

fn run(rx: Receiver<Job>) {
    // Validation admission stays occupied through a credited quarantine.
    let mut retiring: Option<Receiver<Vec<String>>> = None;
    for job in rx {
        if matches!(&job, Job::Batch { cancelled, .. } if cancelled.load(Ordering::Acquire)) {
            continue;
        }
        if let Some(done) = retiring.as_ref() {
            match done.try_recv() {
                Ok(_) | Err(TryRecvError::Disconnected) => retiring = None,
                Err(TryRecvError::Empty) => {
                    refuse(job, "validation isolate is still retiring");
                    continue;
                }
            }
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
                    let outcome = if retiring.is_some() {
                        Err("validation isolate is still retiring".into())
                    } else {
                        validate_one(
                            card.card.js.clone(),
                            card.card.shape,
                            card.siblings.clone(),
                            &mut retiring,
                        )
                        .map_err(|error| format!("prepare {}: {error}", card.card.name))
                    };
                    results.push((card, outcome));
                }
                let _ = reply.send(results);
            }
        }
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
                .map(|card| (card, Err(reason.into())))
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
            *retiring = Some(done_rx);
            Err("validation isolate is still retiring".into())
        }
    }
}
