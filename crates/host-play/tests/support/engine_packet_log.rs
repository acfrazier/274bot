use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use serde::Deserialize;

const MAX_CASE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub struct EnginePacketLog {
    path: PathBuf,
    start: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct EnginePacket {
    pub sequence: u64,
    pub tick: u64,
    pub username: String,
    pub opcode: u8,
    pub name: String,
    pub outcome: String,
    pub reason: String,
    pub handler_accepted: bool,
    pub charged_user_event: bool,
    pub scripts: Vec<EngineScript>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct EngineScript {
    pub outcome: String,
    pub reason: String,
    pub script: String,
    pub state: i32,
}

impl EnginePacketLog {
    pub fn capture(path: PathBuf) -> Result<Self, String> {
        let start = std::fs::metadata(&path)
            .map_err(|error| format!("Engine T packet log {}: {error}", path.display()))?
            .len();
        Ok(Self { path, start })
    }

    pub fn read(&self, account: &str) -> Result<Vec<EnginePacket>, String> {
        let mut file =
            File::open(&self.path).map_err(|error| format!("open Engine T packet log: {error}"))?;
        let end = file
            .metadata()
            .map_err(|error| format!("stat Engine T packet log: {error}"))?
            .len();
        let size = end
            .checked_sub(self.start)
            .ok_or("Engine T packet log was truncated during the case")?;
        if size > MAX_CASE_BYTES {
            return Err(format!(
                "Engine T case packet log exceeds {MAX_CASE_BYTES} bytes"
            ));
        }
        file.seek(SeekFrom::Start(self.start))
            .map_err(|error| format!("seek Engine T packet log: {error}"))?;
        let mut text = String::new();
        file.take(size)
            .read_to_string(&mut text)
            .map_err(|error| format!("read Engine T packet log: {error}"))?;
        parse(&text, account)
    }
}

fn parse(text: &str, account: &str) -> Result<Vec<EnginePacket>, String> {
    if !text.is_empty() && !text.ends_with('\n') {
        return Err("Engine T packet log ends in an incomplete record".into());
    }
    let mut packets = Vec::new();
    let mut previous_sequence = None;
    for (index, line) in text.lines().enumerate() {
        let packet: EnginePacket = serde_json::from_str(line)
            .map_err(|error| format!("Engine T packet record {}: {error}", index + 1))?;
        if previous_sequence
            .is_some_and(|previous: u64| previous.checked_add(1) != Some(packet.sequence))
        {
            return Err(format!(
                "Engine T packet sequence gap before {}",
                packet.sequence
            ));
        }
        previous_sequence = Some(packet.sequence);
        if !matches!(packet.outcome.as_str(), "accepted" | "deferred" | "refused")
            || packet.reason.is_empty()
            || (!packet.handler_accepted && packet.charged_user_event)
            || (!packet.handler_accepted && packet.outcome != "refused")
        {
            return Err(format!("invalid Engine T packet outcome: {packet:?}"));
        }
        if packet.username == account {
            packets.push(packet);
        }
    }
    if packets.is_empty() {
        return Err(format!("Engine T packet log has no records for {account}"));
    }
    Ok(packets)
}

#[cfg(test)]
mod tests {
    use super::parse;

    fn row(sequence: u64, user: &str, accepted: bool, charged: bool, outcome: &str) -> String {
        format!(
            "{{\"sequence\":{sequence},\"tick\":41,\"username\":\"{user}\",\"opcode\":53,\"name\":\"OPHELDU\",\"outcome\":\"{outcome}\",\"reason\":\"guard\",\"handler_accepted\":{accepted},\"charged_user_event\":{charged},\"scripts\":[]}}\n"
        )
    }

    #[test]
    fn separates_handler_charge_from_script_refusal_and_account() {
        let text = row(10, "tmxother", true, true, "accepted")
            + row(11, "tmxmine", true, true, "refused").as_str()
            + row(12, "tmxmine", true, false, "deferred").as_str()
            + row(13, "tmxmine", false, false, "refused").as_str();
        let packets = parse(&text, "tmxmine").unwrap();
        assert_eq!(packets.len(), 3);
        assert!(packets[0].handler_accepted);
        assert!(packets[0].charged_user_event);
        assert_eq!(packets[0].outcome, "refused");
        assert_eq!(packets[1].outcome, "deferred");
        assert!(!packets[2].handler_accepted);
    }

    #[test]
    fn rejects_missing_truncated_and_gapped_records() {
        assert!(parse("", "tmxmine").is_err());
        assert!(parse(&row(10, "tmxother", true, true, "accepted"), "tmxmine").is_err());
        let complete = row(10, "tmxmine", true, true, "accepted");
        assert!(parse(complete.trim_end(), "tmxmine").is_err());
        let gap = complete + row(12, "tmxmine", true, true, "accepted").as_str();
        assert!(parse(&gap, "tmxmine").is_err());
    }

    #[test]
    fn rejects_impossible_charge_and_unknown_outcome() {
        assert!(parse(&row(1, "tmxmine", false, true, "refused"), "tmxmine").is_err());
        assert!(parse(&row(1, "tmxmine", false, false, "deferred"), "tmxmine").is_err());
        assert!(parse(&row(1, "tmxmine", true, true, "queued"), "tmxmine").is_err());
    }
}
