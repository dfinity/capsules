use anyhow::{anyhow, Result};
use assert_cmd;
use std::fs;
use std::io::{self, Write};
use std::net;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::process;
use std::time::SystemTime;
use std::{thread, time};

use nix::unistd;
use rand::Rng;

use tempfile::{self, TempDir};

pub const MINIO_PORT_RANGE: (u16, u16) = (30000, 55000);

pub struct SetupData {
    minio: process::Child,
    pub directory: Option<TempDir>,
    pub port: u16,
}

fn wait_for_port_availability<T>(port: u16, func: fn(u16) -> std::io::Result<T>) -> Result<i32> {
    let max_duration = time::Duration::from_secs(5);
    let start = SystemTime::now();
    let mut busy = true;
    let mut count = 0;
    while busy {
        match func(port) {
            Ok(_) => {
                busy = false;
            }
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => {}
            Err(err) if err.kind() == io::ErrorKind::ConnectionRefused => {}
            Err(err) => return Err(err.into()),
        }
        thread::sleep(time::Duration::from_millis(200));
        if start.elapsed()? > max_duration {
            return Err(anyhow!(
                "Failed to wait for port {} availability after {} steps",
                port,
                count
            ));
        }
        count += 1;
    }
    // After we've waited enough, let's wait some more.
    thread::sleep(time::Duration::from_millis(1000));
    Ok(count)
}

fn wait_for_bind(port: u16) -> Result<()> {
    let count = wait_for_port_availability(port, |port| {
        let result = net::TcpListener::bind(("127.0.0.1", port));
        if let Ok(ref conn) = result {
            // Make sure we close the FD right after we did our check, so that the port
            // is not busy.
            unistd::close(conn.as_raw_fd()).unwrap();
        }
        result
    })?;
    println!("Waiting for bind in {} steps", count);
    Ok(())
}

fn wait_for_connect(port: u16) -> Result<()> {
    let count = wait_for_port_availability(port, |port| net::TcpStream::connect(("127.0.0.1", port)))?;
    println!("Waiting for connect in {} steps", count);
    Ok(())
}

impl Drop for SetupData {
    fn drop(&mut self) {
        println!("Stopping minio");
        self.minio.kill().expect("Failed to stop Minio");
        self.minio.wait().expect("Failed to wait Minio to finish");
        println!("Cleaning up minio directories");
        self.directory.take().map(|d| d.close());
    }
}

impl SetupData {
    pub fn path(&self, elem: &str) -> PathBuf {
        self.directory.as_ref().unwrap().path().join(elem)
    }
}

pub fn setup() -> SetupData {
    let directory = tempfile::tempdir().expect("Failed to create temp dir");
    fs::create_dir_all(directory.path().join("minio").join("capsule-test")).unwrap();
    let mut rng = rand::thread_rng();
    let port = rng.gen_range(MINIO_PORT_RANGE.0..MINIO_PORT_RANGE.1);
    wait_for_bind(port).unwrap();
    // thread::sleep(time::Duration::from_millis(1000));
    let minio = process::Command::new("minio")
        .args([
            "server",
            directory.path().join("minio").to_str().unwrap(),
            "--address",
            &format!("127.0.0.1:{}", port),
            "--quiet",
        ])
        .spawn()
        .expect("Minio failed to start");
    wait_for_connect(port).unwrap();
    SetupData {
        minio,
        directory: Some(directory),
        port,
    }
}

pub fn capsule(port: u16, args: &[&str]) {
    let output = assert_cmd::Command::cargo_bin("capsule")
        .expect("Couldn't find capsule target")
        .env("AWS_ACCESS_KEY_ID", "minioadmin")
        .env("AWS_SECRET_ACCESS_KEY", "minioadmin")
        .env(
            "CAPSULE_ARGS",
            format!(
                "--s3_bucket=capsule-test --s3_bucket_objects=capsule_objects --s3_region=eu-central-1 --s3_endpoint=http://127.0.0.1:{}",
                port
            ),
        )
        .args(args)
        .output()
        .expect("Couldn't execute capsule");
    io::stdout().write_all(&output.stdout).unwrap();
    io::stderr().write_all(&output.stderr).unwrap();
    assert!(output.status.success());
}