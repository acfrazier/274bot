//! Content-derived debugproc and engine-cheat metadata.

use serde::Deserialize;

const MAX_COMMAND_BYTES: usize = 80;

const DEBUG_ARGUMENT_KINDS: &[&str] = &[
    "int",
    "string",
    "obj",
    "npc",
    "loc",
    "seq",
    "spotanim",
    "interface",
    "stat",
    "varp",
    "inv",
    "idkit",
    "namedobj",
    "coord",
];

/// One positional value accepted by a debug command.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DebugArgument {
    pub name: String,
    pub kind: String,
    pub optional: bool,
}

/// One content debugproc or engine cheat exposed by the DebugPanel.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DebugCommand {
    /// The exact wire token. Content debugprocs retain their `~` prefix.
    pub name: String,
    pub category: String,
    pub description: String,
    pub args: Vec<DebugArgument>,
    pub destructive: bool,
    pub production_only: bool,
}

/// One selected-content name usable by a typed debug argument picker.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DebugName {
    pub id: i32,
    pub alias: String,
    pub name: String,
}

impl DebugCommand {
    /// Compose the exact cheat body and reject malformed shapes before it can
    /// reach the single existing CLIENT_CHEAT path.
    pub fn format_command(&self, values: &[String]) -> Result<String, String> {
        if self.name.is_empty() {
            return Err("debug command has no wire name".to_string());
        }
        if !self.name.is_ascii()
            || self
                .name
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(format!("{} has an invalid wire name", self.name));
        }
        for argument in &self.args {
            if !DEBUG_ARGUMENT_KINDS.contains(&argument.kind.as_str()) {
                return Err(format!(
                    "{} has unsupported argument kind {}",
                    self.name, argument.kind
                ));
            }
        }
        if values.len() > self.args.len() {
            return Err(format!(
                "{} expects at most {} argument(s), got {}",
                self.name,
                self.args.len(),
                values.len()
            ));
        }
        let mut value_count = values.len();
        while value_count > 0
            && values[value_count - 1].is_empty()
            && self.args[value_count - 1].optional
        {
            value_count -= 1;
        }
        for (index, argument) in self.args.iter().enumerate() {
            if index >= value_count {
                if !argument.optional {
                    return Err(format!(
                        "{} argument {} is required at position {}",
                        self.name,
                        argument.name,
                        index + 1
                    ));
                }
                continue;
            }
            let value = &values[index];
            if value.is_empty() {
                return Err(format!("{} argument {} is empty", self.name, argument.name));
            }
            if !value.is_ascii()
                || value
                    .chars()
                    .any(|character| character.is_control() || character.is_whitespace())
            {
                return Err(format!(
                    "{} argument {} is not a single ASCII token",
                    self.name, argument.name
                ));
            }
            if argument.kind == "int" && value.parse::<i32>().is_err() {
                return Err(format!(
                    "{} argument {} is not a signed 32-bit integer",
                    self.name, argument.name
                ));
            }
        }
        let mut command = self.name.clone();
        for value in values.iter().take(value_count) {
            command.push(' ');
            command.push_str(value);
        }
        if command.len() > MAX_COMMAND_BYTES {
            return Err(format!(
                "{} is {} bytes; debug cheats are limited to {} bytes",
                self.name,
                command.len(),
                MAX_COMMAND_BYTES
            ));
        }
        Ok(command)
    }
}
#[cfg(test)]
mod tests {
    use super::{DebugArgument, DebugCommand};

    fn command(args: Vec<DebugArgument>) -> DebugCommand {
        DebugCommand {
            name: "~demo".to_string(),
            category: "Account".to_string(),
            description: String::new(),
            args,
            destructive: false,
            production_only: false,
        }
    }

    #[test]
    fn formats_required_and_optional_values_in_wire_order() {
        let debug = command(vec![
            DebugArgument {
                name: "stat".into(),
                kind: "stat".into(),
                optional: false,
            },
            DebugArgument {
                name: "level".into(),
                kind: "int".into(),
                optional: true,
            },
        ]);
        assert_eq!(
            debug.format_command(&["attack".into()]).unwrap(),
            "~demo attack"
        );
        assert_eq!(
            debug
                .format_command(&["attack".into(), "42".into()])
                .unwrap(),
            "~demo attack 42"
        );
        assert_eq!(
            debug
                .format_command(&["attack".into(), String::new()])
                .unwrap(),
            "~demo attack"
        );
        assert!(debug.format_command(&[]).is_err());
        assert!(debug
            .format_command(&["attack".into(), "42".into(), "extra".into()])
            .is_err());
    }

    #[test]
    fn rejects_unknown_kind_empty_values_and_line_breaks() {
        let unknown = command(vec![DebugArgument {
            name: "value".into(),
            kind: "future".into(),
            optional: true,
        }]);
        assert!(unknown.format_command(&[]).is_err());
        let debug = command(vec![DebugArgument {
            name: "value".into(),
            kind: "string".into(),
            optional: false,
        }]);
        assert!(debug.format_command(&[" ".into()]).is_err());
        assert!(debug.format_command(&["line\nbreak".into()]).is_err());
    }

    #[test]
    fn rejects_invalid_integer_tokens_and_optional_gaps() {
        let integer = command(vec![DebugArgument {
            name: "value".into(),
            kind: "int".into(),
            optional: false,
        }]);
        assert!(integer.format_command(&["not-int".into()]).is_err());
        assert!(integer.format_command(&["1 2".into()]).is_err());
        assert!(integer.format_command(&["é".into()]).is_err());
        let gap = command(vec![
            DebugArgument {
                name: "obj".into(),
                kind: "obj".into(),
                optional: false,
            },
            DebugArgument {
                name: "count".into(),
                kind: "int".into(),
                optional: true,
            },
            DebugArgument {
                name: "tag".into(),
                kind: "string".into(),
                optional: true,
            },
        ]);
        assert!(gap
            .format_command(&["coins".into(), String::new(), "x".into()])
            .is_err());
    }

    #[test]
    fn enforces_eighty_byte_wire_limit_not_character_count() {
        let debug = command(vec![DebugArgument {
            name: "value".into(),
            kind: "string".into(),
            optional: false,
        }]);
        let short = "a".repeat(74);
        assert!(debug.format_command(&[short]).is_ok());
        let multibyte = "é".repeat(40);
        assert!(debug.format_command(&[multibyte]).is_err());
    }
}
