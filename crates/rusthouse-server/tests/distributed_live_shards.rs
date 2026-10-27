//! Exercise the real coordinator, TCP client and immutable shard readers.
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

static NEXT_DB: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
struct Harness {
    children: Vec<Child>,
    directories: Vec<PathBuf>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        for path in &self.directories {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn database(column: &str, rows: &[(u64, &str)]) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "rusthouse-distributed-live-{}-{}",
        std::process::id(),
        NEXT_DB.fetch_add(1, Ordering::Relaxed)
    ));
    let part = directory.join("events.parts/all_1_1_0");
    fs::create_dir_all(part.parent().unwrap()).unwrap();
    let schema = format!("rusthouse-schema-v1\nid\tUInt64\n{column}\tString\n");
    fs::write(directory.join("events.schema"), schema).unwrap();
    let columns = [
        Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: column.into(),
            data_type: "String".into(),
        },
    ];
    let values: Vec<Vec<Option<String>>> = rows
        .iter()
        .map(|(id, kind)| vec![Some(id.to_string()), Some((*kind).to_owned())])
        .collect();
    write_wide_part(&part, &columns, &values).unwrap();
    directory
}

/// Reserve all ports simultaneously so none are duplicated in a test.
fn ports() -> [u16; 3] {
    let listeners: Vec<_> = (0..3)
        .map(|_| TcpListener::bind(("127.0.0.1", 0)).unwrap())
        .collect();
    let ports = [
        listeners[0].local_addr().unwrap().port(),
        listeners[1].local_addr().unwrap().port(),
        listeners[2].local_addr().unwrap().port(),
    ];
    drop(listeners);
    ports
}

fn request(port: u16, query: Option<&str>) -> io::Result<(String, Vec<u8>)> {
    let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(300))?;
    stream.set_read_timeout(Some(Duration::from_secs(4)))?;
    stream.set_write_timeout(Some(Duration::from_secs(4)))?;
    if let Some(query) = query {
        write!(
            stream,
            "POST /distributed HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            query.len()
        )?;
        stream.write_all(query.as_bytes())?;
    } else {
        stream.write_all(b"GET /ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    }
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let end = response
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .ok_or_else(|| io::Error::other("invalid HTTP response"))?;
    let headers = std::str::from_utf8(&response[..end])
        .map_err(|_| io::Error::other("invalid HTTP headers"))?;
    let status = headers
        .lines()
        .next()
        .ok_or_else(|| io::Error::other("missing HTTP status"))?
        .to_owned();
    Ok((status, response[end + 4..].to_vec()))
}

fn wait_ready(port: u16) {
    for _ in 0..150 {
        if let Ok((status, body)) = request(port, None) {
            if status.starts_with("HTTP/1.1 200") && body == b"Ok.\n" {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("rusthouse-server never became ready on port {port}");
}

fn spawn(
    harness: &mut Harness,
    port: u16,
    database: Option<&Path>,
    peers: &[u16],
    max_rows: usize,
) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusthouse-server"));
    command
        .arg("--port")
        .arg(port.to_string())
        .arg("--max-rows")
        .arg(max_rows.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(database) = database {
        command.arg("--database").arg(database);
    }
    for &peer in peers {
        command.arg("--peer").arg(format!("127.0.0.1:{peer}"));
    }
    harness.children.push(command.spawn().unwrap());
    wait_ready(port);
}

#[test]
fn live_shards_merge_scans_counts_and_groups_and_fail_closed_on_peer_loss() {
    let mut harness = Harness::default();
    let shard_a = database("kind", &[(1, "red"), (2, "blue")]);
    let shard_b = database("kind", &[(3, "blue"), (4, "red")]);
    harness
        .directories
        .extend([shard_a.clone(), shard_b.clone()]);
    let [a, b, coordinator] = ports();
    spawn(&mut harness, a, Some(&shard_a), &[], 100);
    spawn(&mut harness, b, Some(&shard_b), &[], 100);
    spawn(&mut harness, coordinator, None, &[a, b], 100);

    let (status, rows) = request(coordinator, Some("SELECT id, kind FROM events")).unwrap();
    assert!(status.contains(" 200 "), "{status}: {rows:?}");
    assert_eq!(rows, b"id\tkind\n1\tred\n2\tblue\n3\tblue\n4\tred\n");

    let (status, count) = request(coordinator, Some("SELECT count(*) FROM events")).unwrap();
    assert!(status.contains(" 200 "), "{status}: {count:?}");
    assert_eq!(count, b"count\n4\n");

    let (status, groups) = request(
        coordinator,
        Some("SELECT kind, count(*) FROM events GROUP BY kind"),
    )
    .unwrap();
    assert!(status.contains(" 200 "), "{status}: {groups:?}");
    let groups = String::from_utf8(groups).unwrap();
    let mut lines = groups.lines().skip(1).collect::<Vec<_>>();
    lines.sort_unstable();
    assert_eq!(lines, ["blue\t2", "red\t2"]);

    let (status, _) = request(coordinator, Some("SELECT DISTINCT id FROM events")).unwrap();
    assert!(status.contains(" 400 "), "{status}");

    let mut stopped = harness.children.remove(1);
    stopped.kill().unwrap();
    stopped.wait().unwrap();
    let (status, _) = request(coordinator, Some("SELECT count(*) FROM events")).unwrap();
    assert!(status.contains(" 502 "), "{status}");
}

#[test]
fn live_shards_reject_incompatible_output_headers() {
    let mut harness = Harness::default();
    let a_db = database("kind", &[(1, "red")]);
    let b_db = database("label", &[(2, "blue")]);
    harness.directories.extend([a_db.clone(), b_db.clone()]);
    let [a, b, coordinator] = ports();
    spawn(&mut harness, a, Some(&a_db), &[], 100);
    spawn(&mut harness, b, Some(&b_db), &[], 100);
    spawn(&mut harness, coordinator, None, &[a, b], 100);
    let (status, body) = request(coordinator, Some("SELECT * FROM events")).unwrap();
    assert!(status.contains(" 409 "), "{status}: {body:?}");
}

#[test]
fn live_shards_reject_capped_partial_scans() {
    let mut harness = Harness::default();
    let db = database("kind", &[(1, "red"), (2, "blue")]);
    harness.directories.push(db.clone());
    let [a, _, coordinator] = ports();
    spawn(&mut harness, a, Some(&db), &[], 2);
    spawn(&mut harness, coordinator, None, &[a], 100);
    let (status, body) = request(coordinator, Some("SELECT id FROM events")).unwrap();
    assert!(status.contains(" 409 "), "{status}: {body:?}");
}
