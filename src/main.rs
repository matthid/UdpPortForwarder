use std::env;
use std::net::{UdpSocket, SocketAddr};
use std::thread;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::io;

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: UdpPortForwarder <target_ip> <port1> <port2> ...");
        return Ok(());
    }

    let target_ip = &args[1];
    let ports = &args[2..];
    let client_map: Arc<Mutex<HashMap<SocketAddr, SocketAddr>>> = Arc::new(Mutex::new(HashMap::new()));

    for port in ports {
        let local_addr = format!("0.0.0.0:{}", port);
        let target_addr = format!("{}:{}", target_ip, port);
        println!("Listen to {} -> {}", local_addr, target_addr);
        let listen_socket = UdpSocket::bind(&local_addr).expect("Could bind to listen-socket");
        listen_socket.set_nonblocking(true).expect("Could not set listen-socket to non-blocking");

        let listen_socket_reply = listen_socket.try_clone().expect("Could not clone listen-socket");

        let forward_socket = UdpSocket::bind("0.0.0.0:0")
            .expect("Could bind to forward-socket"); // Bind to a random free port
        forward_socket.set_nonblocking(true).expect("Could not set forward-socket to non-blocking");

        let target_addr: SocketAddr = target_addr.parse().expect("Invalid target address");
        let mapt1 = client_map.clone();
        let mapt2 = client_map.clone();
        let forward_socket_clone = forward_socket.try_clone().expect("Could not clone forward-socket");

        // Thread for forwarding requests to the target
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match listen_socket.recv_from(&mut buf) {
                    Ok((size, src)) => {
                        mapt1.lock().unwrap().insert(target_addr, src);
                        if let Err(e) = forward_socket.send_to(&buf[..size], target_addr) {
                            eprintln!("Failed to forward packet: {}", e);
                        }
                    }
                    Err(ref e) if e.kind() != io::ErrorKind::WouldBlock => {
                        eprintln!("Failed to receive packet: {}", e);
                    }
                    _ => {}
                }
            }
        });

        // Thread for forwarding responses back to clients
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match forward_socket_clone.recv_from(&mut buf) {
                    Ok((size, _)) => {
                        if let Some(&client) = mapt2.lock().unwrap().get(&target_addr) {
                            if let Err(e) = listen_socket_reply.send_to(&buf[..size], client) {
                                eprintln!("Failed to send response: {}", e);
                            }
                        }
                    }
                    Err(ref e) if e.kind() != io::ErrorKind::WouldBlock => {
                        eprintln!("Failed to receive response: {}", e);
                    }
                    _ => {}
                }
            }
        });
    }

    // Keep the main thread alive
    loop {
        thread::sleep(std::time::Duration::from_secs(1));
    }
}
