//! Estado compartido entre el hilo que ejecuta una operación y quien la observa.
//!
//! Los contadores son atómicos porque se leen varias veces por segundo desde la
//! interfaz y no deben bloquear al hilo que copia. Lo que sí necesita cerrojo
//! —textos, errores, el conflicto pendiente— cambia pocas veces.

use crate::model::*;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

/// Lo que se decide ante un destino ocupado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    KeepBoth,
    Skip,
    Overwrite,
    Cancel,
}

impl From<ConflictDecision> for Resolution {
    fn from(d: ConflictDecision) -> Self {
        match d {
            ConflictDecision::KeepBoth => Resolution::KeepBoth,
            ConflictDecision::Skip => Resolution::Skip,
            ConflictDecision::Overwrite => Resolution::Overwrite,
            ConflictDecision::Cancel => Resolution::Cancel,
        }
    }
}

#[derive(Default)]
struct Slow {
    current: String,
    errors: Vec<String>,
    pending: Option<Conflict>,
    answer: Option<Resolution>,
    /// Decisión que el usuario marcó como «aplicar a todos».
    sticky: Option<Resolution>,
    actions: Vec<Action>,
    changes: Vec<PathChange>,
}

pub struct OpShared {
    pub id: u64,
    pub kind: OpKind,
    started: Instant,
    state: AtomicU8,
    cancelled: AtomicBool,
    undoable: AtomicBool,
    total_items: AtomicU64,
    done_items: AtomicU64,
    total_bytes: AtomicU64,
    done_bytes: AtomicU64,
    slow: Mutex<Slow>,
    /// La petición original, por si hay que **rehacer** la operación.
    original: Mutex<Option<OpRequest>>,
    /// Despierta al hilo que espera una decisión sobre un conflicto.
    answered: Condvar,
}

fn state_to_u8(s: OpState) -> u8 {
    match s {
        OpState::Planning => 0,
        OpState::Running => 1,
        OpState::WaitingConflict => 2,
        OpState::Cancelling => 3,
        OpState::Done => 4,
        OpState::Cancelled => 5,
        OpState::Failed => 6,
    }
}

fn u8_to_state(v: u8) -> OpState {
    match v {
        0 => OpState::Planning,
        1 => OpState::Running,
        2 => OpState::WaitingConflict,
        3 => OpState::Cancelling,
        4 => OpState::Done,
        5 => OpState::Cancelled,
        _ => OpState::Failed,
    }
}

impl OpShared {
    pub fn new(id: u64, kind: OpKind) -> Self {
        Self {
            id,
            kind,
            started: Instant::now(),
            state: AtomicU8::new(state_to_u8(OpState::Planning)),
            cancelled: AtomicBool::new(false),
            undoable: AtomicBool::new(true),
            total_items: AtomicU64::new(0),
            done_items: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
            done_bytes: AtomicU64::new(0),
            slow: Mutex::new(Slow::default()),
            original: Mutex::new(None),
            answered: Condvar::new(),
        }
    }

    pub fn state(&self) -> OpState {
        u8_to_state(self.state.load(Ordering::SeqCst))
    }

    pub fn set_state(&self, s: OpState) {
        self.state.store(state_to_u8(s), Ordering::SeqCst);
    }

    pub fn set_totals(&self, items: u64, bytes: u64) {
        self.total_items.store(items, Ordering::Relaxed);
        self.total_bytes.store(bytes, Ordering::Relaxed);
    }

    pub fn advance_bytes(&self, n: u64) {
        self.done_bytes.fetch_add(n, Ordering::Relaxed);
    }

    pub fn advance_item(&self, path: &Path) {
        self.done_items.fetch_add(1, Ordering::Relaxed);
        self.set_current(path);
    }

    pub fn advance_bulk(&self, items: u64, bytes: u64, path: &Path) {
        self.done_items.fetch_add(items.max(1), Ordering::Relaxed);
        self.done_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.set_current(path);
    }

    pub fn set_current(&self, path: &Path) {
        if let Ok(mut slow) = self.slow.lock() {
            slow.current = path.to_string_lossy().to_string();
        }
    }

    pub fn push_error(&self, msg: String) {
        tracing::warn!("operación {}: {msg}", self.id);
        if let Ok(mut slow) = self.slow.lock() {
            // Un error por archivo en una copia de diez mil archivos llenaría la
            // memoria sin ayudar a nadie: se guardan los primeros.
            if slow.errors.len() < 200 {
                slow.errors.push(msg);
            }
        }
    }

    pub fn has_errors(&self) -> bool {
        self.slow.lock().map(|s| !s.errors.is_empty()).unwrap_or(false)
    }

    pub fn record(&self, action: Action) {
        if let Ok(mut slow) = self.slow.lock() {
            slow.actions.push(action);
        }
    }

    pub fn notify(&self, change: PathChange) {
        if let Ok(mut slow) = self.slow.lock() {
            slow.changes.push(change);
        }
    }

    pub fn take_changes(&self) -> Vec<PathChange> {
        self.slow
            .lock()
            .map(|mut s| std::mem::take(&mut s.changes))
            .unwrap_or_default()
    }

    pub fn mark_not_undoable(&self) {
        self.undoable.store(false, Ordering::SeqCst);
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if !self.state().is_finished() {
            self.set_state(OpState::Cancelling);
        }
        // Si estaba esperando una respuesta sobre un conflicto, se le da una.
        if let Ok(mut slow) = self.slow.lock()
            && slow.pending.is_some()
        {
            slow.answer = Some(Resolution::Cancel);
        }
        self.answered.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn sticky_resolution(&self) -> Option<Resolution> {
        self.slow.lock().ok().and_then(|s| s.sticky)
    }

    /// Publica el conflicto y **bloquea** este hilo hasta que alguien responda.
    ///
    /// El hilo de la interfaz sigue vivo: ve el estado `WaitingConflict` y el
    /// conflicto pendiente, y responde con `answer`.
    pub fn ask(&self, conflict: Conflict) -> Resolution {
        let mut slow = match self.slow.lock() {
            Ok(g) => g,
            Err(_) => return Resolution::Cancel,
        };
        slow.pending = Some(conflict);
        slow.answer = None;
        drop(slow);

        let anterior = self.state();
        self.set_state(OpState::WaitingConflict);

        let mut slow = match self.slow.lock() {
            Ok(g) => g,
            Err(_) => return Resolution::Cancel,
        };
        while slow.answer.is_none() {
            if self.cancelled.load(Ordering::SeqCst) {
                slow.pending = None;
                return Resolution::Cancel;
            }
            slow = match self.answered.wait(slow) {
                Ok(g) => g,
                Err(_) => return Resolution::Cancel,
            };
        }

        let decision = slow.answer.take().unwrap_or(Resolution::Cancel);
        slow.pending = None;
        drop(slow);
        self.set_state(anterior);
        decision
    }

    /// Responde a un conflicto pendiente. `apply_to_all` fija la decisión para
    /// el resto de la operación.
    pub fn answer(&self, decision: ConflictDecision, apply_to_all: bool) {
        let resolution = Resolution::from(decision);
        if let Ok(mut slow) = self.slow.lock() {
            slow.answer = Some(resolution);
            if apply_to_all {
                slow.sticky = Some(resolution);
            }
        }
        if resolution == Resolution::Cancel {
            self.cancelled.store(true, Ordering::SeqCst);
        }
        self.answered.notify_all();
    }

    pub fn progress(&self) -> OpProgress {
        let (current, errors, pending) = match self.slow.lock() {
            Ok(s) => (s.current.clone(), s.errors.clone(), s.pending.clone()),
            Err(_) => (String::new(), Vec::new(), None),
        };
        OpProgress {
            id: self.id,
            kind: self.kind,
            state: self.state(),
            total_items: self.total_items.load(Ordering::Relaxed),
            done_items: self.done_items.load(Ordering::Relaxed),
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            done_bytes: self.done_bytes.load(Ordering::Relaxed),
            current,
            errors,
            pending_conflict: pending,
            can_undo: self.undoable.load(Ordering::SeqCst) && self.state() == OpState::Done,
            elapsed_ms: self.started.elapsed().as_millis() as u64,
        }
    }

    /// Recuerda la petición original para poder rehacerla.
    pub fn set_original(&self, req: OpRequest) {
        if let Ok(mut o) = self.original.lock() {
            *o = Some(req);
        }
    }

    pub fn take_original(&self) -> Option<OpRequest> {
        self.original.lock().ok().and_then(|mut o| o.take())
    }

    pub fn receipt(&self) -> Receipt {
        let actions = self
            .slow
            .lock()
            .map(|s| s.actions.clone())
            .unwrap_or_default();
        Receipt {
            id: self.id,
            kind: self.kind,
            undoable: self.undoable.load(Ordering::SeqCst) && !actions.is_empty(),
            actions,
        }
    }
}
