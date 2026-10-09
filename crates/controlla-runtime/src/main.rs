use std::path::PathBuf;

fn usage() {
    println!(
        "Chrome Controlla\n\nUsage: controlla [--state-dir DIR] <command>\n\nCommands:\n  doctor       Report local configuration and health evidence\n  schema       Print the capability schema\n  mcp          Run the local MCP stdio server\n  serve-http   Run the authenticated loopback MCP HTTP server\n  install-bridge EXTENSION_ID  Register Chrome's native messaging host\n  help         Show this help"
    );
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    if args
        .peek()
        .is_some_and(|arg| arg.starts_with("chrome-extension://"))
    {
        let origin = args.next().unwrap();
        let origin = origin.strip_suffix('/').unwrap_or(&origin);
        if origin.len() != "chrome-extension://".len() + 32
            || !origin["chrome-extension://".len()..]
                .bytes()
                .all(|b| (b'a'..=b'p').contains(&b))
        {
            invalid("invalid native host origin");
        }
        let runtime = tokio::runtime::Runtime::new().expect("native host runtime");
        if let Err(error) = runtime.block_on(controlla_runtime::native_host::run()) {
            eprintln!("Chrome native host failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if args
        .peek()
        .is_some_and(|arg| arg == "--__controlla-script-worker")
    {
        args.next();
        if args.next().is_some() {
            invalid("unexpected worker argument");
        }
        if let Err(error) = controlla_runtime::workflow::run_script_worker() {
            eprintln!("Script worker failed: {error}");
            std::process::exit(1);
        }
        return;
    }
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
    if !matches!(
        command.as_str(),
        "help" | "doctor" | "schema" | "mcp" | "serve-http" | "install-bridge"
    ) {
        invalid(&format!("unknown command: {command}"));
    }
    if args
        .peek()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        usage();
        return;
    }
    let extension_id = if command == "install-bridge" {
        Some(
            args.next()
                .unwrap_or_else(|| invalid("install-bridge requires the exact extension ID")),
        )
    } else {
        None
    };
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
        "serve-http" => {
            if let Err(error) = controlla_runtime::http_server::run(state_override) {
                eprintln!("HTTP MCP server failed: {error}");
                std::process::exit(1);
            }
        }
        "install-bridge" => {
            match controlla_runtime::native_setup::install_host(&extension_id.unwrap()) {
                Ok(path) => println!("Installed Chrome native host: {}", path.display()),
                Err(error) => invalid(&error),
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
