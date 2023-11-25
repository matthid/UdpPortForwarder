use tokio::net::UdpSocket;
use tokio::time::{self, Duration, Instant};
use std::env;
use std::collections::HashMap;
use std::net::{SocketAddr};
use std::ops::{AddAssign, Deref, DerefMut};
use std::sync::{Arc};
use std::thread::sleep;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::{io, signal};
use tokio::sync::broadcast::error::SendError;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

pub struct Client {
    listen_socket: Arc<UdpSocket>,
    target_addr: SocketAddr,
    client_addr: SocketAddr,
    tx: mpsc::Sender<Vec<u8>>,
    last_data_timestamp: Arc<Mutex<Instant>>,
    should_terminate: broadcast::Sender<()>,  // Cancellation flag
    back_channel_handler: Mutex<Option<JoinHandle<io::Result<()>>>>,
    forward_channel_handler: Mutex<Option<JoinHandle<io::Result<()>>>>
    // ... other client-related state ...
}

impl Client {
    pub async fn new(listen_socket: Arc<UdpSocket>, target_addr: SocketAddr, client_addr: SocketAddr) -> Self {
        // ... initialize other state ...

        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(100);

        let last_data_timestamp = Arc::new(Mutex::new(Instant::now()));

        let (terminate_tx, _) = broadcast::channel(10); // Create a broadcast channel

        let client_socket = Arc::new(UdpSocket::bind("0.0.0.0:0").await.expect("failed to bind UDP socket"));
        println!("Handling new client {} <-> {} <=> {} <-> {}",
                 client_addr, listen_socket.local_addr().expect("Expected listen_socket.local_addr()"),
                 client_socket.local_addr().expect("Expected client_socket.local_addr()"), target_addr);

        // Backchannel
        let mut back_stop = terminate_tx.subscribe();
        let client_socket_read = client_socket.clone();
        let listen_socket_back = listen_socket.clone();
        let last_data_timestamp_back = last_data_timestamp.clone();
        let backward_handler = tokio::spawn(async move {
            let mut back_buf = [0u8; 4096];
            loop {
                tokio::select! {
                    data = client_socket_read.recv_from(&mut back_buf) => {
                        //println!("Received response from {}", client_socket_read.local_addr().expect("expected local address"));
                        match &data
                        {
                            io::Result::Ok((size, sender)) => {
                                let size = size.clone();
                                Client::update_timestamp(&last_data_timestamp_back).await;
                                //println!("Received response from {} ({} bytes)", client_socket_read.local_addr().expect("expected local address"), size);
                                if let Err(e) = listen_socket_back.send_to(&back_buf[..size], client_addr).await {
                                    eprintln!("Failed to send response back to {} ({} bytes)", client_addr, size);
                                }
                            },
                            io::Result::Err(e) => {
                                eprintln!("Error receiving data on back-channel: {}", e);
                            }
                        }
                    }
                    _ = back_stop.recv() => {
                        println!("Stopping back-channel {}", client_addr);
                        return io::Result::Ok(());
                    }
                }
            }
        });

        // Forward channel
        let mut forward_stop = terminate_tx.subscribe();
        let last_data_timestamp_forward = last_data_timestamp.clone();
        let forward_handler = tokio::spawn(async move {
            loop {
                let data = tokio::select! {
                    data = rx.recv() => {
                        Client::update_timestamp(&last_data_timestamp_forward).await;
                        if data.is_none() {
                            // Client channel closed
                            println!("Client channel closed for {}", client_addr);
                            return Ok(());
                        }
                        data.unwrap()
                    }
                    _ = forward_stop.recv() => {
                        println!("Stopping forwardchannel {}", client_addr);
                        return Ok(());
                    }
                };

                //println!("Sending {} bytes of data to {}", data.len(), target_addr);
                client_socket.send_to(&data, target_addr).await?;
            }
        });

        Client {
            listen_socket,
            target_addr,
            client_addr,
            tx,
            last_data_timestamp,
            should_terminate: terminate_tx,
            back_channel_handler: Mutex::new(Some(backward_handler)),
            forward_channel_handler: Mutex::new(Some(forward_handler)),
        }
    }

    async fn send(&self, data: Vec<u8>) -> Result<(), tokio::sync::mpsc::error::SendError<Vec<u8>>> {
        self.tx.send(data).await
    }

    async fn update_timestamp(timestamp: &Arc<Mutex<Instant>>) {
        let mut timestamp = timestamp.lock().await;
        *timestamp = Instant::now();
    }

    pub async fn handle_timeout(&self, timeout: Duration) -> bool {
        if self.is_timed_out(timeout).await {
            println!("Timeout {}", self.client_addr);
            if let Err(e)  = self.signal_termination() {
                eprintln!("Signal_termination in handle_timeout failed: {}", e);
            }
            true
        } else {
            false
        }
    }

    async fn is_timed_out(&self, timeout_duration: Duration) -> bool {
        let timestamp = self.last_data_timestamp.lock().await;
        timestamp.elapsed() > timeout_duration
    }

    fn signal_termination(&self) -> Result<usize, SendError<()>> {
       self.should_terminate.send(())
    }

    pub async fn terminate(&self) -> io::Result<()> {
        println!("Terminate {}", self.client_addr);
        self.signal_termination().expect("Signal_termination failed in terminate");

        {
            let mut guard = self.back_channel_handler.lock().await;
            if let Some(d) =guard.deref_mut(){
                d.await??
            }
            *guard = None;
        }

        {
            let mut guard = self.forward_channel_handler.lock().await;
            if let Some(d) =guard.deref_mut(){
                d.await??
            }
            *guard = None;
        }
        Ok(())
    }
}


pub struct ClientInitialize {
    listen_socket: Arc<UdpSocket>,
    target_addr: SocketAddr,
    client_addr: SocketAddr
}

pub enum ClientInit {
    Created(ClientInitialize),
    Initialized(Arc<Client>)
}
pub struct ClientInitializeHandle {
    client_state: Arc<Mutex<ClientInit>>
}

impl ClientInitializeHandle {
    pub fn new(listen_socket: Arc<UdpSocket>, target_addr: SocketAddr, client_addr: SocketAddr) -> Self {
        ClientInitializeHandle {
            client_state: Arc::new(Mutex::new(ClientInit::Created(ClientInitialize{
                listen_socket,
                target_addr,
                client_addr
            })))
        }
    }

    pub async fn get_client(&mut self) -> Arc<Client> {
        let mut guard = self.client_state.lock().await;
        match guard.deref_mut() {
            ClientInit::Created(c) => {
                let client = Arc::new(Client::new(c.listen_socket.clone(), c.target_addr, c.client_addr).await);
                *guard = ClientInit::Initialized(client.clone());
                return client;
            },
            ClientInit::Initialized(client) => {
                return client.clone();
            }
        }
    }
}

struct PortListener {
    join: JoinHandle<io::Result<()>>
}

impl PortListener {

    pub fn new(listen_socket: Arc<UdpSocket>, target_addr: SocketAddr, client_map: Arc<Mutex<HashMap<SocketAddr, ClientInitializeHandle>>>, mut terminate_receiver: broadcast::Receiver<()>) -> Self {

        let handle = tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                tokio::select! {
                    rec_data = listen_socket.recv_from(&mut buf) => {
                        match &rec_data {
                            Ok((size, src)) => {
                                let src = src.clone();
                                let size = size.clone();
                                let data = buf[..size].to_vec();

                                //println!("Receive {} bytes of data from {}", size, src);

                                let mut map = client_map.lock().await;

                                let mut client_handle = map.entry(src).or_insert_with(|| {
                                    ClientInitializeHandle::new(listen_socket.clone(), target_addr.clone(), src)
                                });

                                let client = client_handle.get_client().await;

                                if let Err(e) = client.send(data).await {
                                    eprintln!("Failed to send packet to forwarding task: {}", e);
                                }
                            },
                            Result::Err(e) => {
                                eprintln!("Failed to receive packet: {}", e);
                            },
                        }
                    }
                    _ = terminate_receiver.recv() => {
                        println!("Stopping port listener on {}", listen_socket.local_addr().expect("Expected listen_socket.local_addr() in PortListener"));
                        return Ok(())
                    }
                }
            }
        });

        PortListener {
            join: handle
        }
    }

}

#[tokio::main]
async fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: UdpPortForwarder <target_ip> <port1> <port2> ...");
        return Ok(());
    }

    let target_ip = &args[1];
    let ports = &args[2..];
    let client_map: Arc<Mutex<HashMap<SocketAddr, ClientInitializeHandle>>> = Arc::new(Mutex::new(HashMap::new()));

    let (terminate_listener, _) = broadcast::channel(10); // Create a broadcast channel
    let mut handles: Vec<PortListener> = Vec::new();
    for port in ports {
        let local_addr = format!("0.0.0.0:{}", port);
        println!("Listening on {} -> {}:{}", local_addr, target_ip, port);
        let listen_socket = Arc::new(UdpSocket::bind(&local_addr).await.expect("failed to bind UDP socket"));
        let target_addr = format!("{}:{}", target_ip, port);
        let listener = PortListener::new(listen_socket, target_addr.parse().unwrap(), client_map.clone(), terminate_listener.subscribe());
        handles.push(listener);
    }

    let mut timeout_stop = terminate_listener.subscribe();
    let timeout_client_map = client_map.clone();
    let handle_timeouts = tokio::spawn(async move {
        let timeout_duration = Duration::from_secs(60);
        loop {
            tokio::select! {
                _ = tokio::time::sleep(timeout_duration.clone()) => {
                    let mut guard = timeout_client_map.lock().await;
                    let mut keys: Vec<SocketAddr> = Vec::new();
                    {
                        for socket in guard.keys() {
                            keys.push(socket.clone())
                        }
                    }

                    for socket_to_remove in keys {
                        if let Some(client) = guard.get_mut(&socket_to_remove){
                            if client.get_client().await.handle_timeout(timeout_duration.clone()).await {
                                guard.remove(&socket_to_remove);
                            }
                        }
                    }
                }
                _ = timeout_stop.recv() => {
                    println!("Stopping timeout handler");
                    return ()
                }
            }
        }
    });


    // Handle CTRL+C signal
    signal::ctrl_c().await.expect("Failed to listen for ctrl_c");
    println!("Sending termination signal!");
    if let Err(e) = terminate_listener.send(()) {
        eprintln!("Failed to terminate listener: {}", e);
    }
    for client in client_map.lock().await.values_mut() {
        client.get_client().await.terminate().await?;
    }

    // Await all the spawned tasks
    for handle in handles {
        if let Err(e) = handle.join.await {
            eprintln!("Task ended with error: {:?}", e);
        }
    }

    Ok(())
}
