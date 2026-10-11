//! Bounded auxiliary processes. Never leave an ffprobe/version child behind.
use super::*;
use std::process::Output;

type Children = HashMap<u32, Arc<Mutex<Child>>>;
fn children() -> &'static Mutex<Children> {
    static CHILDREN: OnceLock<Mutex<Children>> = OnceLock::new();
    CHILDREN.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn stop_all() {
    if let Ok(children) = children().lock() {
        for child in children.values() {
            let _ = stop_child_process_group(child);
        }
    }
}

fn read_bounded(reader: impl Read) -> Vec<u8> {
    // Drain the pipe even after the cap so a noisy process cannot deadlock.
    let mut result = Vec::new();
    let mut reader = reader;
    let mut buffer = [0; 8192];
    while let Ok(count) = reader.read(&mut buffer) {
        if count == 0 {
            break;
        }
        let keep = count.min((1024 * 1024_usize).saturating_sub(result.len()));
        result.extend_from_slice(&buffer[..keep]);
    }
    result
}

fn pipe_reader(reader: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = send.send(read_bounded(reader));
    });
    receive
}

fn finish_reader(
    reader: Option<std::sync::mpsc::Receiver<Vec<u8>>>,
    deadline: Instant,
) -> std::io::Result<Vec<u8>> {
    match reader {
        None => Ok(Vec::new()),
        Some(reader) => reader
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Допоміжний процес не закрив канал виводу",
                )
            }),
    }
}

pub(crate) fn capture(command: &mut Command, timeout: Duration) -> std::io::Result<Output> {
    configure_command(command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().map(pipe_reader);
    let stderr = child.stderr.take().map(pipe_reader);
    let pid = child.id();
    let child = Arc::new(Mutex::new(child));
    children()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(pid, child.clone());
    let deadline = Instant::now() + timeout;
    let result = loop {
        let status = child.lock().unwrap_or_else(|e| e.into_inner()).try_wait();
        match status {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = stop_child_process_group(&child);
                let _ = child.lock().unwrap_or_else(|e| e.into_inner()).wait();
                break Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Допоміжний процес не відповів вчасно",
                ));
            }
            Err(error) => {
                let _ = stop_child_process_group(&child);
                break Err(error);
            }
        }
    };
    children()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&pid);
    // An inherited pipe must not hang the caller after its owned child exited.
    // Never signal a reaped PID: it may already belong to another process.
    let status = result?;
    let drain_deadline = Instant::now() + Duration::from_secs(1);
    let stdout = finish_reader(stdout, drain_deadline)?;
    let stderr = finish_reader(stderr, drain_deadline)?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caps_output_and_drains_input() {
        assert_eq!(
            read_bounded(&vec![42; 2 * 1024 * 1024][..]).len(),
            1024 * 1024
        );
    }
    #[cfg(unix)]
    #[test]
    fn times_out_a_child_group() {
        let now = Instant::now();
        let error = capture(
            Command::new("sh").args(["-c", "sleep 10"]),
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(now.elapsed() < Duration::from_secs(3));
    }
}
