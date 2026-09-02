//! Canal de control con el servicio de indexado.
//!
//! La aplicación y el servicio son dos procesos distintos con privilegios
//! distintos: la aplicación solo lee el índice, y el servicio es el único que lo
//! escribe. Todo lo que la aplicación necesita pedirle —excluir un volumen,
//! comunicar el estado de la licencia, avisar de un cambio que acaba de hacer—
//! pasa por aquí.
//!
//! Ninguna llamada es obligatoria para buscar: si el servicio no responde, la
//! aplicación sigue funcionando en solo lectura sobre el índice que haya.

use bdj_search_ipc::{IpcCommand, IpcEvent};

/// Envía una orden y espera la respuesta.
///
/// Devuelve `None` cuando no hay servicio al otro lado, que es un estado normal
/// —el servicio puede estar arrancando, o el usuario puede no haberlo instalado
/// todavía— y no un error que deba interrumpir nada.
#[cfg(windows)]
pub fn send(cmd: &IpcCommand) -> Option<IpcEvent> {
    use bdj_search_ipc::{DEFAULT_PIPE_NAME, PipeClient};
    let client = PipeClient::connect(DEFAULT_PIPE_NAME).ok()?;
    client.send_command(cmd).ok()?;
    client.read_event().ok()
}

#[cfg(unix)]
pub fn send(cmd: &IpcCommand) -> Option<IpcEvent> {
    use bdj_search_ipc::{default_unix_socket_path, UnixSocketClient};
    let mut client = UnixSocketClient::connect(default_unix_socket_path()).ok()?;
    client.send_command(cmd).ok()?;
    client.read_event().ok()
}

#[cfg(not(any(windows, unix)))]
pub fn send(_cmd: &IpcCommand) -> Option<IpcEvent> {
    None
}

/// ¿Aceptó el servicio la orden?
pub fn send_ok(cmd: &IpcCommand) -> bool {
    matches!(send(cmd), Some(IpcEvent::Ack))
}
