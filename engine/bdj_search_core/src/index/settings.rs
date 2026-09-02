//! Ajustes persistentes del servicio de indexado.
//!
//! Viven junto al índice, en `settings.json`, y sobreviven al reinicio del
//! servicio y a la recompactación del índice. Se identifican los volúmenes por
//! su **prefijo de montaje** y no por el identificador numérico interno, porque
//! ese número se reasigna cada vez que el índice se reconstruye: guardar «el
//! volumen 3 está excluido» acabaría excluyendo otro disco.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IndexerSettings {
    /// Indexado activo en general.
    #[serde(default = "verdadero")]
    pub indexing_enabled: bool,
    /// Volúmenes que el usuario ha excluido, por prefijo de montaje.
    #[serde(default)]
    pub excluded_volumes: BTreeSet<String>,
    /// Carpetas sueltas añadidas al índice.
    #[serde(default)]
    pub extra_folders: BTreeSet<String>,
    /// Última licencia comunicada por la aplicación.
    #[serde(default)]
    pub license_active: bool,
    /// Momento de caducidad, en segundos desde 1970. Cero si no caduca.
    #[serde(default)]
    pub license_expires_at: u64,
}

fn verdadero() -> bool {
    true
}

impl IndexerSettings {
    pub fn new() -> Self {
        Self {
            indexing_enabled: true,
            ..Default::default()
        }
    }

    /// Lee los ajustes. Un archivo ausente o corrupto devuelve los de fábrica:
    /// el servicio nunca debe quedarse sin arrancar por un ajuste ilegible.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            // Un archivo ilegible no puede impedir que el servicio arranque:
            // se vuelve a los ajustes de fábrica y el primer `save` lo repara.
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| Self::new()),
            Err(_) => Self::new(),
        }
    }

    /// Escribe los ajustes de forma atómica: temporal y renombrado encima.
    ///
    /// Un corte de corriente en mitad de la escritura dejaría, si no, un archivo
    /// truncado que en el siguiente arranque se leería como «sin ajustes».
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }

    /// ¿Hay que indexar este volumen?
    pub fn is_volume_indexed(&self, mount_prefix: &str) -> bool {
        !self
            .excluded_volumes
            .iter()
            .any(|v| v.eq_ignore_ascii_case(mount_prefix))
    }

    pub fn set_volume_indexed(&mut self, mount_prefix: &str, indexed: bool) {
        self.excluded_volumes
            .retain(|v| !v.eq_ignore_ascii_case(mount_prefix));
        if !indexed {
            self.excluded_volumes.insert(mount_prefix.to_string());
        }
    }

    pub fn add_folder(&mut self, path: &str) {
        self.extra_folders.insert(path.to_string());
    }

    pub fn remove_folder(&mut self, path: &str) {
        self.extra_folders
            .retain(|f| !f.eq_ignore_ascii_case(path));
    }

    /// ¿Puede el servicio seguir indexando?
    ///
    /// Sin licencia activa, no. Una licencia caducada tampoco sirve, aunque la
    /// aplicación la diera por buena en su momento: el servicio arranca solo con
    /// el sistema y puede llevar meses sin que nadie abra la aplicación.
    pub fn may_index(&self, now_secs: u64) -> bool {
        if !self.indexing_enabled {
            return false;
        }
        if !self.license_active {
            return false;
        }
        self.license_expires_at == 0 || now_secs < self.license_expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn los_ajustes_van_y_vuelven_del_disco() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");

        let mut s = IndexerSettings::new();
        s.set_volume_indexed("D:\\", false);
        s.add_folder("C:\\Musica\\Sets");
        s.license_active = true;
        s.license_expires_at = 1_900_000_000;
        s.save(&p).unwrap();

        let leidos = IndexerSettings::load(&p);
        assert_eq!(leidos, s);
        assert!(!leidos.is_volume_indexed("d:\\"));
        assert!(leidos.is_volume_indexed("C:\\"));
    }

    #[test]
    fn un_archivo_corrupto_no_impide_arrancar() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, b"{ esto no es json").unwrap();
        let s = IndexerSettings::load(&p);
        assert!(s.indexing_enabled);
        assert!(s.excluded_volumes.is_empty());
    }

    #[test]
    fn un_archivo_ausente_da_los_ajustes_de_fabrica() {
        let dir = tempdir().unwrap();
        let s = IndexerSettings::load(&dir.path().join("no-existe.json"));
        assert!(s.indexing_enabled);
    }

    #[test]
    fn volver_a_incluir_un_volumen_lo_saca_de_la_lista() {
        let mut s = IndexerSettings::new();
        s.set_volume_indexed("E:\\", false);
        assert!(!s.is_volume_indexed("E:\\"));
        s.set_volume_indexed("E:\\", true);
        assert!(s.is_volume_indexed("E:\\"));
        assert!(s.excluded_volumes.is_empty());
    }

    #[test]
    fn sin_licencia_no_se_indexa() {
        let mut s = IndexerSettings::new();
        assert!(!s.may_index(1_800_000_000), "de fábrica no hay licencia");

        s.license_active = true;
        assert!(s.may_index(1_800_000_000));

        s.license_expires_at = 1_700_000_000;
        assert!(
            !s.may_index(1_800_000_000),
            "una licencia caducada no habilita el indexado"
        );

        s.license_expires_at = 1_900_000_000;
        assert!(s.may_index(1_800_000_000));

        s.indexing_enabled = false;
        assert!(!s.may_index(1_800_000_000));
    }
}
