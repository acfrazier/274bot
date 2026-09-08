use crate::core::*;
use serde_json::{Value, json};
pub fn run(case: &str) -> Result<Value> {
    match case {
        "abnormal" => std::process::exit(2),
        "incomplete" => std::process::exit(0),
        "malformed" => {
            println!("not JSON");
            std::process::exit(0);
        }
        "duplicate" => {
            let v = json!({"phase":1,"wall_start":mono(),"cpu_start":cpu()});
            send(&v)?;
            send(&v)?;
            std::process::exit(0);
        }
        _ => {}
    }
    let over = match case {
        "cpu" => json!({"cpu":1,"wall":5}),
        "wall" | "phase_end" => json!({"wall":1}),
        "rss" => json!({"rss":32*MIB}),
        "address" => json!({"address":64*MIB}),
        _ => return Err("unknown finite guard case"),
    };
    let limits = Limits::new(&over)?;
    crate::contract::set_limits(&limits)?;
    let mut g = Guard::new(limits, true);
    g.next(1)?;
    match case {
        "cpu" => {
            while mono() - g.start < 4.0 {
                g.pulse(0)?;
            }
        }
        "wall" | "phase_end" => {
            std::thread::sleep(std::time::Duration::from_secs(2));
            g.next(2)?;
        }
        "rss" | "address" => {
            let n = if case == "rss" { 64 * MIB } else { 96 * MIB };
            let mut data = vec![0u8; n];
            for i in (0..n).step_by(4096) {
                unsafe {
                    std::ptr::write_volatile(data.as_mut_ptr().add(i), 1);
                }
            }
            g.check()?;
            std::thread::sleep(std::time::Duration::from_secs(1));
            std::hint::black_box(data);
        }
        _ => unreachable!(),
    }
    Err("negative guard did not fire")
}
