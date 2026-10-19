//! WebSocket message admission at the binary game-byte boundary.
//!
//! Tungstenite assembles fragments and queues automatic Pong/Close replies;
//! the reactor explicitly flushes after every read, including WouldBlock.
//! No control frame or raw wire byte is written to the game ring.

use tungstenite::Message;

use super::ring::{Ring, IN_RING_BYTES, WS_MESSAGE_LIMIT};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FrameFault {
    MessageTooLarge { length: usize, limit: usize },
    UnexpectedText,
    UnexpectedRawFrame,
}

#[derive(Debug)]
pub enum WsReceive {
    /// The whole binary message entered the game ring once.
    Binary(usize),
    /// Inbound high-water. The reactor retains this *moved* message until
    /// ResumeRead; no new socket read or copy is allowed in the meantime.
    Backpressured(Vec<u8>),
    /// Auto-generated control output may await explicit WS flush.
    Control,
    PeerClosed { code: Option<u16>, reason: String },
}

/// Called exactly once for each successful `ws.read()` result. An empty
/// Binary is a valid no-op, not EOF. Oversize and Text fail this connection
/// without exposing a prefix of the rejected payload.
pub fn admit_message(
    message: Message,
    inbound: &Ring<IN_RING_BYTES>,
) -> Result<WsReceive, FrameFault> {
    match message {
        Message::Binary(bytes) => {
            if bytes.len() > WS_MESSAGE_LIMIT {
                return Err(FrameFault::MessageTooLarge {
                    length: bytes.len(),
                    limit: WS_MESSAGE_LIMIT,
                });
            }
            if bytes.is_empty() {
                return Ok(WsReceive::Binary(0));
            }
            if inbound.push_all(&bytes) {
                Ok(WsReceive::Binary(bytes.len()))
            } else {
                Ok(WsReceive::Backpressured(bytes))
            }
        }
        Message::Ping(_) | Message::Pong(_) => Ok(WsReceive::Control),
        Message::Close(frame) => Ok(WsReceive::PeerClosed {
            code: frame.as_ref().map(|value| value.code.into()),
            reason: frame.map_or_else(String::new, |value| value.reason.into_owned()),
        }),
        Message::Text(_) => Err(FrameFault::UnexpectedText),
        Message::Frame(_) => Err(FrameFault::UnexpectedRawFrame),
    }
}

/// Reattempt a retained message after the slot has drained ring space. This
/// does not call `ws.read()` or resubmit a tungstenite data Message.
pub fn admit_pending(
    message: Vec<u8>,
    inbound: &Ring<IN_RING_BYTES>,
) -> WsReceive {
    if inbound.push_all(&message) {
        WsReceive::Binary(message.len())
    } else {
        WsReceive::Backpressured(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tungstenite::protocol::frame::coding::CloseCode;
    use tungstenite::protocol::CloseFrame;

    #[test]
    fn controls_and_text_never_enter_game_ring() {
        let ring = Ring::new();
        assert!(matches!(admit_message(Message::Ping(b"private".to_vec()), &ring), Ok(WsReceive::Control)));
        assert!(matches!(admit_message(Message::Pong(b"private".to_vec()), &ring), Ok(WsReceive::Control)));
        assert_eq!(admit_message(Message::Text("private".into()), &ring).unwrap_err(), FrameFault::UnexpectedText);
        assert_eq!(ring.len(), 0);
        let close = Message::Close(Some(CloseFrame { code: CloseCode::Normal, reason: "bye".into() }));
        assert!(matches!(admit_message(close, &ring), Ok(WsReceive::PeerClosed { code: Some(1000), .. })));
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn binary_high_water_retains_entire_message_once() {
        let ring = Ring::new();
        assert!(ring.push_all(&vec![3; IN_RING_BYTES - 2]));
        let pending = match admit_message(Message::Binary(vec![7; 3]), &ring).unwrap() {
            WsReceive::Backpressured(bytes) => bytes,
            other => panic!("expected backpressure, got {other:?}"),
        };
        assert_eq!(ring.len(), IN_RING_BYTES - 2);
        let mut drained = vec![0; 3];
        assert_eq!(ring.pop(&mut drained), 3);
        assert!(matches!(admit_pending(pending, &ring), WsReceive::Binary(3)));
        let mut rest = vec![0; IN_RING_BYTES - 2];
        assert_eq!(ring.pop(&mut rest), IN_RING_BYTES - 2);
        assert_eq!(&rest[IN_RING_BYTES - 5..], &[7; 3]);
    }

    #[test]
    fn oversized_binary_refuses_before_copy() {
        let ring = Ring::new();
        assert_eq!(
            admit_message(Message::Binary(vec![1; WS_MESSAGE_LIMIT + 1]), &ring).unwrap_err(),
            FrameFault::MessageTooLarge { length: WS_MESSAGE_LIMIT + 1, limit: WS_MESSAGE_LIMIT },
        );
        assert_eq!(ring.len(), 0);
    }
}
