use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::str;

use revisor_common::util;

struct Args {
    help: bool,
    control_socket: String,
    free_args: Vec<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(()) => ExitCode::FAILURE,
    }
}

fn run() -> Result<(), ()> {
    let args = parse_args()?;
    if args.help {
        return Ok(());
    }

    let mut stream = UnixStream::connect(&args.control_socket).map_err(|err| {
        eprintln!("Error: Failed to connect control socket: {}", err);
    })?;

    let msg: String = args.free_args.join("\t");
    writeln!(stream, "{}", msg).map_err(|err| {
        eprintln!("Error: Failed to write to control socket: {}", err);
    })?;

    let mut response_buf = [0u8; 256];
    let mut response_offset = 0;
    while response_offset < response_buf.len() {
        let bytes_written = stream.read(&mut response_buf[response_offset..]).map_err(|err| {
            eprintln!("Error: Failed to read response from control socket: {}", err);
        })?;
        response_offset += bytes_written;
        if bytes_written == 0 {
            break;
        }
        if response_buf[response_offset - 1] == b'\n' {
            break;
        }
    }

    for ch in &mut response_buf {
        if *ch == b'\t' {
            *ch = b' ';
        }
    }

    let response = str::from_utf8(&response_buf).map_err(|err| {
        eprintln!("Error: Failed to parse response: {}", err);
    })?;
    eprint!("{}", response);

    Ok(())
}

impl TryFrom<getopts::Matches> for Args {
    type Error = ();
    fn try_from(matches: getopts::Matches) -> Result<Args, ()> {
        Ok(Args {
            help: matches.opt_present("help"),
            control_socket: matches.opt_str("control-socket").expect("required option missing"),
            free_args: matches.free,
        })
    }
}

impl util::Args for Args {
    fn need_help(&self) -> bool {
        self.help
    }
}

fn parse_args() -> Result<Args, ()> {
    let mut opts = getopts::Options::new();
    opts.reqopt(
        "s",
        "control-socket",
        "path to control socket (default: path/to/units/control.sock)",
        "CONTROL_SOCK",
    );
    util::parse_args("Usage: rvctl -s path/to/units/control.sock command [command options]", opts)
}
