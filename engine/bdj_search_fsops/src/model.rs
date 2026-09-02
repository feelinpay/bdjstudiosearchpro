use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Qué se le pide al gestor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpKind {
    /// Crear una carpeta vacía dentro del destino.
    CreateFolder,
    /// Crear un archivo vacío dentro del destino.
    CreateFile,
    /// Cambiar el nombre de una única entrada.
    Rename,
    Copy,
    Move,
    /// Copiar junto al original con un nombre libre («… (2)»).
    Duplicate,
    /// A la papelera del sistema. Nunca borra de forma definitiva.
    Trash,
    /// Devolver un elemento de la papelera a su ubicación original.
    Restore,
    /// Borrar del disco sin pasar por la papelera. **No se puede deshacer.**
    DeletePermanently,
}

impl OpKind {
    /// Cierto si la operación puede modificar archivos que ya existían.
    ///
    /// De ella depende que se pregunte por los conflictos.
    pub fn can_conflict(self) -> bool {
        matches!(
            self,
            OpKind::CreateFolder | OpKind::CreateFile | OpKind::Rename | OpKind::Copy | OpKind::Move
        )
    }
}

/// Qué hacer cuando el destino ya existe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ConflictPolicy {
    /// Detener la operación y preguntar. Es lo predeterminado.
    #[default]
    Ask,
    /// Conservar los dos, añadiendo un sufijo al nuevo.
    KeepBoth,
    Skip,
    /// Sobrescribir. Deja la operación **fuera del alcance de deshacer**: lo que
    /// se pisa no se puede recuperar.
    Overwrite,
}

/// Respuesta del usuario a un conflicto concreto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictDecision {
    KeepBoth,
    Skip,
    Overwrite,
    /// Abandonar toda la operación.
    Cancel,
}

/// Un choque concreto entre lo que se quiere escribir y lo que ya hay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub source: String,
    pub destination: String,
    pub source_size: u64,
    pub destination_size: u64,
    pub source_mtime: u32,
    pub destination_mtime: u32,
    pub destination_is_dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpState {
    /// Contando archivos y bytes antes de empezar.
    Planning,
    Running,
    /// Detenida a la espera de que el usuario resuelva un conflicto.
    WaitingConflict,
    Cancelling,
    Done,
    Cancelled,
    Failed,
}

impl OpState {
    pub fn is_finished(self) -> bool {
        matches!(self, OpState::Done | OpState::Cancelled | OpState::Failed)
    }
}

/// Lo que se le pide al gestor, ya validado.
#[derive(Debug, Clone)]
pub struct OpRequest {
    pub kind: OpKind,
    pub sources: Vec<PathBuf>,
    /// Carpeta de destino en copiar y mover; ruta final en renombrar; carpeta
    /// contenedora en crear carpeta.
    pub destination: Option<PathBuf>,
    /// Nombre de la carpeta nueva, o el nombre nuevo al renombrar.
    pub new_name: Option<String>,
    pub conflict: ConflictPolicy,
}

impl OpRequest {
    pub fn new(kind: OpKind, sources: Vec<PathBuf>) -> Self {
        Self {
            kind,
            sources,
            destination: None,
            new_name: None,
            conflict: ConflictPolicy::Ask,
        }
    }

    pub fn with_destination(mut self, dest: PathBuf) -> Self {
        self.destination = Some(dest);
        self
    }

    pub fn with_new_name(mut self, name: String) -> Self {
        self.new_name = Some(name);
        self
    }

    pub fn with_policy(mut self, policy: ConflictPolicy) -> Self {
        self.conflict = policy;
        self
    }
}

/// Foto del estado de una operación, tal y como la ve la interfaz.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpProgress {
    pub id: u64,
    pub kind: OpKind,
    pub state: OpState,
    pub total_items: u64,
    pub done_items: u64,
    pub total_bytes: u64,
    pub done_bytes: u64,
    /// Ruta que se está procesando ahora mismo.
    pub current: String,
    pub errors: Vec<String>,
    pub pending_conflict: Option<Conflict>,
    /// Cierto cuando la operación se puede deshacer entera.
    pub can_undo: bool,
    pub elapsed_ms: u64,
}

/// Lo que realmente se hizo, paso a paso.
///
/// Es lo que permite deshacer con precisión: no se reconstruye la intención,
/// se invierte cada acción concreta que llegó a ejecutarse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Action {
    Created { path: String, is_dir: bool },
    Moved { from: String, to: String },
    Copied { to: String },
    Trashed { original: String },
    /// Vuelto a su ubicación original desde la papelera.
    Restored { path: String },
    /// Se pisó algo que ya existía: a partir de aquí no hay vuelta atrás.
    Overwrote { path: String },
    /// Borrado del disco de forma definitiva: no hay papelera que lo recupere.
    Deleted { path: String },
}

/// Registro completo de una operación terminada.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub id: u64,
    pub kind: OpKind,
    pub actions: Vec<Action>,
    pub undoable: bool,
}

/// Aviso de que la aplicación tocó el disco por su cuenta.
///
/// El servicio lo usa para actualizar el índice al instante y para **no**
/// procesar dos veces el evento del sistema de archivos que llega justo después.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PathChange {
    Created { path: String, is_dir: bool },
    Removed { path: String },
    Renamed { from: String, to: String },
}

impl PathChange {
    /// Rutas afectadas, para la ventana de supresión.
    pub fn paths(&self) -> Vec<&str> {
        match self {
            PathChange::Created { path, .. } | PathChange::Removed { path } => vec![path.as_str()],
            PathChange::Renamed { from, to } => vec![from.as_str(), to.as_str()],
        }
    }
}
