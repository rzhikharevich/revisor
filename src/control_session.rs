use std::io;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::str;

use crate::poll::PollEvents;
use crate::sys;
use crate::unit_manager;

pub struct ControlSession {
    stream: UnixStream,
    input_buf_offset: usize,
    input_buf: [u8; 256],
    output_buf_offset: usize,
    output_buf: [u8; 256],
}

const MAX_RESPONSE_LEN: usize = 256;

impl ControlSession {
    pub fn new(stream: UnixStream) -> ControlSession {
        ControlSession {
            stream,
            input_buf_offset: 0,
            input_buf: [0; 256],
            output_buf_offset: 0,
            output_buf: [0; 256],
        }
    }

    pub fn poll_output(&mut self) -> Result<(), ()> {
        if self.output_buf_offset > 0 {
            let write_len = match sys::retry_eintr(|| {
                self.stream.write(&self.output_buf[..self.output_buf_offset])
            }) {
                Ok(write_len) => write_len,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(err) => {
                    eprintln!("Error writing to unix stream: {}", err);
                    return Err(());
                }
            };
            self.output_buf.copy_within(write_len..self.output_buf_offset, 0);
            self.output_buf_offset -= write_len;
        }
        Ok(())
    }

    pub fn poll_input(&mut self, unit_manager: &mut unit_manager::UnitManager) -> Result<(), ()> {
        if self.output_buf_is_full() {
            return Ok(());
        }

        if self.input_buf_offset >= self.input_buf.len() {
            return Err(());
        }
        let read_len = match sys::retry_eintr(|| {
            self.stream.read(&mut self.input_buf[self.input_buf_offset..])
        }) {
            Ok(read_len) => read_len,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(err) => {
                eprintln!("Error reading from unix stream: {}", err);
                return Err(());
            }
        };
        if read_len == 0 {
            return Err(());
        }
        let read_end = self.input_buf_offset + read_len;

        let mut command_start = 0;
        while let Some(relative_lf_offset) =
            self.input_buf[command_start..read_end].iter().position(|c| *c == '\n' as u8)
        {
            let lf_offset = command_start + relative_lf_offset;
            self.process_command(command_start, lf_offset, unit_manager);
            command_start = lf_offset + 1;

            if self.output_buf_is_full() {
                break;
            }
        }

        self.input_buf.copy_within(command_start..read_end, 0);
        self.input_buf_offset = read_end - command_start;

        if self.input_buf_offset == self.input_buf.len() {
            self.write_response("error: command too long");
            return Err(());
        }

        Ok(())
    }

    pub fn events(&self) -> PollEvents {
        let mut events = PollEvents::NONE;
        if self.output_buf_offset > 0 {
            events |= PollEvents::OUTPUT;
        }
        if !self.output_buf_is_full() {
            events |= PollEvents::INPUT;
        }
        events
    }

    fn output_buf_is_full(&self) -> bool {
        self.output_buf_offset > self.output_buf.len() - MAX_RESPONSE_LEN
    }

    fn process_command(
        &mut self,
        command_start: usize,
        command_end: usize,
        unit_manager: &mut unit_manager::UnitManager,
    ) {
        let Ok(command) = str::from_utf8(&self.input_buf[command_start..command_end]) else {
            return;
        };
        if command.is_empty() {
            return;
        }

        let mut tokens = command.split(|c| c == '\t');
        let verb = tokens.next().expect("empty command split");

        let mut cursor = io::Cursor::new(&mut self.output_buf[..]);
        cursor.set_position(self.output_buf_offset as u64);

        let write_result = match verb {
            "start" => {
                if let Some(unit_name) = tokens.next() {
                    match unit_manager.start(unit_name) {
                        Ok(_pid) => writeln!(cursor, "ok"),
                        Err(err) => writeln!(cursor, "error: {}", err),
                    }
                } else {
                    writeln!(cursor, "error: expected a unit name after 'start'")
                }
            }
            "stop" => {
                if let Some(unit_name) = tokens.next() {
                    match unit_manager.stop(unit_name) {
                        Ok(()) => writeln!(cursor, "ok"),
                        Err(err) => writeln!(cursor, "error: {}", err),
                    }
                } else {
                    writeln!(cursor, "error: expected a unit name after 'stop'")
                }
            }
            "status" => {
                if let Some(unit_name) = tokens.next() {
                    if let Some(status) = unit_manager.status(unit_name) {
                        write!(cursor, "ok: ")
                            .and_then(|_| serialize_unit_status(&status, &mut cursor))
                            .and_then(|_| writeln!(cursor))
                    } else {
                        writeln!(cursor, "error: not found")
                    }
                } else {
                    writeln!(cursor, "error: expected a unit name after 'status'")
                }
            }
            _ => writeln!(cursor, "unkn"),
        };

        write_result.expect("failed to write response");
        self.output_buf_offset = cursor.position() as usize;
    }

    fn write_response(&mut self, response: &str) {
        let mut cursor = io::Cursor::new(&mut self.output_buf[..]);
        cursor.set_position(self.output_buf_offset as u64);
        writeln!(cursor, "{}", response).expect("failed to write response");
        self.output_buf_offset = cursor.position() as usize;
    }
}

impl AsRawFd for ControlSession {
    fn as_raw_fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }
}

fn serialize_unit_status(
    status: &unit_manager::UnitStatus,
    fmt: &mut impl io::Write,
) -> io::Result<()> {
    if let Some(pid) = status.pid {
        write!(fmt, "pid={}\t", pid)?;
    } else {
        write!(fmt, "pid=dead\t")?;
    }

    write!(fmt, "want_alive={}", status.want_alive)
}
