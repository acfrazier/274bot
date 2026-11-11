use rs2vm::program::{Operand, Program};
use rs2vm::{ScriptId, Site};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
const USAGE: &str = "usage: rs2vm dis ENGINE CONTENT CLIENT_VARBIT_DAT SCRIPT_ID_OR_NAME";

fn main() -> ExitCode {
    match run(env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err(USAGE.to_owned());
    };
    if command.as_os_str() != OsStr::new("dis") {
        return Err(USAGE.to_owned());
    }

    let engine = required_path(&mut args, "ENGINE")?;
    let content = required_path(&mut args, "CONTENT")?;
    let client_varbits_path = required_path(&mut args, "CLIENT_VARBIT_DAT")?;
    let query = args
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(|| format!("{USAGE}\nSCRIPT_ID_OR_NAME must be valid UTF-8"))?;
    if args.next().is_some() {
        return Err(USAGE.to_owned());
    }

    let client_varbits = fs::read(&client_varbits_path)
        .map_err(|error| format!("could not read {}: {error}", client_varbits_path.display()))?;
    let program =
        Program::load(&engine, &content, &client_varbits).map_err(|error| error.to_string())?;
    let id = resolve_script(&program, &query)?;
    let script = program
        .script(id)
        .ok_or_else(|| format!("script {} disappeared after lookup", id.0))?;

    println!(
        "script {} {:?} source={} lookup={} args=int:{} string:{} locals=int:{} string:{}",
        id.0,
        script.name,
        script.source,
        script.lookup,
        script.int_args,
        script.string_args,
        script.int_locals,
        script.string_locals,
    );
    for (index, instruction) in script.code.iter().enumerate() {
        let pc = u32::try_from(index).map_err(|_| "program counter exceeds u32".to_owned())?;
        let opcode = program
            .opcode(instruction.opcode)
            .ok_or_else(|| format!("opcode {} has no engine name", instruction.opcode))?;
        let operand = match &instruction.operand {
            Operand::Int(value) => value.to_string(),
            Operand::Text(value) => format!("{value:?}"),
        };
        println!(
            "{}: {opcode} {operand}",
            program.format_site(Site { script: id, pc })
        );
    }
    Ok(())
}

fn required_path(args: &mut impl Iterator<Item = OsString>, name: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{USAGE}\nmissing {name}"))
}

fn resolve_script(program: &Program, query: &str) -> Result<ScriptId, String> {
    if let Ok(raw_id) = query.parse::<u32>() {
        let id = ScriptId(raw_id);
        return program
            .script(id)
            .map(|_| id)
            .ok_or_else(|| format!("no script with id {raw_id}"));
    }

    let mut matching_id = None;
    for script in program.scripts.values() {
        if script.name.as_ref() == query {
            if matching_id.is_some() {
                return Err(format!(
                    "script name {query:?} is ambiguous; use a numeric id"
                ));
            }
            matching_id = Some(script.id);
        }
    }
    matching_id.ok_or_else(|| format!("no script named {query:?}"))
}
