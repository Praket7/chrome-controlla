use std::path::PathBuf;

fn usage() {
    println!(
        "Chrome Controlla\n\nUsage: controlla [--state-dir DIR] <command>\n\nCommands:\n  doctor       Report local configuration and health evidence\n  schema       Print the capability schema\n  mcp          Run the local MCP stdio server\n  help         Show this help"
    );
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut state_override = None;
    while let Some(arg) = args.peek() {
        if arg == "--help" || arg == "-h" {
            usage();
            return;
        }
        if arg == "--version" || arg == "-V" {
            println!("controlla {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        if arg == "--state-dir" {
            args.next();
            let Some(value) = args.next() else {
                invalid("--state-dir requires a directory");
            };
            state_override = Some(PathBuf::from(value));
            continue;
        }
        if arg.starts_with('-') {
            invalid(&format!("unknown argument: {arg}"));
        }
        break;
    }
    let command = args.next().unwrap_or_else(|| "help".to_owned());
    if !matches!(command.as_str(), "help" | "doctor" | "schema" | "mcp") {
        invalid(&format!("unknown command: {command}"));
    }
    if args
        .peek()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        usage();
        return;
    }
    if args.next().is_some() {
        invalid("unexpected argument");
    }
    match command.as_str() {
        "help" => usage(),
        "mcp" => {
            if let Err(error) = controlla_runtime::mcp::run() {
                eprintln!("MCP server failed: {error}");
                std::process::exit(1);
            }
        }
        "schema" => println!("{}", include_str!("../schemas/capabilities.json")),
        "doctor" => {
            let state = state_override
                .or_else(|| std::env::var_os("CONTROLLA_STATE_DIR").map(PathBuf::from))
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(|home| PathBuf::from(home).join(".chrome-controlla"))
                })
                .unwrap_or_else(|| PathBuf::from(".chrome-controlla"));
            let report = controlla_runtime::doctor::report(&state, env!("CARGO_PKG_VERSION"));
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        }
        _ => unreachable!("validated command"),
    }
}

fn invalid(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2)
}
