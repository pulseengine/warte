//! `warte-check <architecture|temper-report> <file>`: check one report.
//! Exit 0 when it is valid, 1 when it is refused, 2 on a usage or read error.
//! Built for wasm32-wasip2 this is a WASI command component; the file is read
//! through a preopened directory, and exit code 2 needs the host's
//! exit-with-code (`wasmtime run -S cli-exit-with-code --dir=.`).

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [kind, path] = args.as_slice() else {
        eprintln!("usage: warte-check <architecture|temper-report> <file>");
        return 2;
    };
    if !matches!(kind.as_str(), "architecture" | "temper-report") {
        eprintln!("warte-check: unknown report kind (architecture, temper-report)");
        return 2;
    }
    let Ok(bytes) = std::fs::read(path) else {
        eprintln!("warte-check: the file cannot be read");
        return 2;
    };
    let errors: Vec<String> = if kind == "architecture" {
        match std::str::from_utf8(&bytes) {
            Ok(text) => warte_check::architecture::check(text)
                .err()
                .unwrap_or_default(),
            Err(_) => vec!["not UTF-8".into()],
        }
    } else {
        warte_check::temper_report::parse(&bytes)
            .err()
            .into_iter()
            .collect()
    };
    if errors.is_empty() {
        println!("{kind}: valid");
        0
    } else {
        for e in &errors {
            println!("{kind}: {e}");
        }
        1
    }
}
