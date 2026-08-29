use bdj_search_ipc::{IpcCommand, IpcEvent, PipeClient, PipeServer};
use std::thread;
use std::time::Duration;

#[test]
fn test_named_pipe_ipc_roundtrip() {
    let pipe_name = r"\\.\pipe\BDJSearchProTestPipe";

    let server_handle = thread::spawn(move || {
        let server = PipeServer::create(pipe_name).expect("Server creation must succeed");
        server.wait_for_client().expect("Client wait must succeed");

        // Receive Command from client
        let cmd = server.read_command().expect("Server read command");
        assert_eq!(cmd, IpcCommand::RescanVolume { volume_id: 2 });

        // Send Event back to client
        let evt = IpcEvent::IndexUpdated {
            generation: 15,
            entry_count: 500_000,
        };
        server.send_event(&evt).expect("Server send event");
        thread::sleep(Duration::from_millis(100));
        server.disconnect();
    });

    // Give server thread 50 ms to start listening
    thread::sleep(Duration::from_millis(50));

    let client = PipeClient::connect(pipe_name).expect("Client connect must succeed");

    // Client sends Command
    let cmd = IpcCommand::RescanVolume { volume_id: 2 };
    client.send_command(&cmd).expect("Client send command");

    // Client reads Event
    let evt = client.read_event().expect("Client read event");
    assert_eq!(
        evt,
        IpcEvent::IndexUpdated {
            generation: 15,
            entry_count: 500_000,
        }
    );

    server_handle.join().unwrap();
    println!("Named Pipe IPC roundtrip test succeeded!");
}
