use bdj_search_fsops::*;
use std::fs;
use std::time::Duration;
use tempfile::tempdir;

const TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn prueba_completa_flujo_real_operaciones_archivos() {
    let base_dir = tempdir().expect("crear directorio temporal");
    let base_path = base_dir.path().to_path_buf();
    let manager = FileOpManager::new();

    // 1. CREAR CARPETA
    let carpeta_nombre = "Sesion_DJ_2026";
    let carpeta_path = base_path.join(carpeta_nombre);
    let id_crear_carpeta = manager.submit(
        OpRequest::new(OpKind::CreateFolder, vec![])
            .with_destination(base_path.clone())
            .with_new_name(carpeta_nombre.to_string()),
    );
    let prog_carpeta = manager.wait_for(id_crear_carpeta, TIMEOUT).expect("esperar crear carpeta");
    assert_eq!(prog_carpeta.state, OpState::Done, "La creación de carpeta debe completarse");
    assert!(carpeta_path.exists() && carpeta_path.is_dir(), "La carpeta debe existir físicamente en disco");

    // 2. CREAR ARCHIVO Y ESCRIBIR CONTENIDO
    let id_crear_archivo = manager.submit(
        OpRequest::new(OpKind::CreateFile, vec![])
            .with_destination(carpeta_path.clone())
            .with_new_name("intro.wav".to_string()),
    );
    let prog_archivo = manager.wait_for(id_crear_archivo, TIMEOUT).expect("esperar crear archivo");
    assert_eq!(prog_archivo.state, OpState::Done, "La creación de archivo debe completarse");
    let archivo_intro = carpeta_path.join("intro.wav");
    assert!(archivo_intro.exists(), "El archivo intro.wav debe existir");
    fs::write(&archivo_intro, b"RIFF....WAVEfmt ....data....BDJ STUDIO PRO TRACK DATA").unwrap();

    // 3. COPIAR RUTA Y VERIFICAR
    let ruta_copiada = archivo_intro.to_string_lossy().to_string();
    assert!(ruta_copiada.ends_with("intro.wav"));
    assert!(fs::metadata(&ruta_copiada).is_ok(), "La ruta copiada debe ser accesible en el sistema");

    // 4. DUPLICAR
    let id_duplicar = manager.submit(
        OpRequest::new(OpKind::Duplicate, vec![archivo_intro.clone()]),
    );
    let prog_duplicar = manager.wait_for(id_duplicar, TIMEOUT).expect("esperar duplicar archivo");
    assert_eq!(prog_duplicar.state, OpState::Done, "La duplicación debe completarse");

    let duplicado_path = carpeta_path.join("intro (2).wav");
    assert!(duplicado_path.exists(), "El archivo duplicado 'intro (2).wav' debe existir");
    assert_eq!(fs::read(&duplicado_path).unwrap(), fs::read(&archivo_intro).unwrap(), "El contenido duplicado debe ser idéntico");

    // 5. RENOMBRAR
    let nuevo_nombre = "intro_extended_mix.wav";
    let id_renombrar = manager.submit(
        OpRequest::new(OpKind::Rename, vec![duplicado_path.clone()])
            .with_new_name(nuevo_nombre.to_string()),
    );
    let prog_renombrar = manager.wait_for(id_renombrar, TIMEOUT).expect("esperar renombrar");
    assert_eq!(prog_renombrar.state, OpState::Done, "El renombrado debe completarse");
    let renombrado_path = carpeta_path.join(nuevo_nombre);
    assert!(!duplicado_path.exists(), "El nombre anterior no debe existir");
    assert!(renombrado_path.exists(), "El nuevo nombre debe existir físicamente");

    // 6. CONVERTIR EN ZIP (COMPRIMIR)
    let zip_dest = base_path.join("Sesion_Exportada.zip");
    let id_zip = manager.submit(
        OpRequest::new(OpKind::CompressZip, vec![carpeta_path.clone()])
            .with_destination(zip_dest.clone()),
    );
    let prog_zip = manager.wait_for(id_zip, TIMEOUT).expect("esperar compresión ZIP");
    assert_eq!(prog_zip.state, OpState::Done, "La compresión ZIP debe completarse");
    assert!(zip_dest.exists(), "El archivo .zip debe existir");
    assert!(fs::metadata(&zip_dest).unwrap().len() > 0, "El ZIP debe contener bytes");

    // 6b. DESCOMPRIMIR ZIP
    let extract_dir = base_path.join("Descomprimido");
    fs::create_dir_all(&extract_dir).unwrap();
    let id_unzip = manager.submit(
        OpRequest::new(OpKind::ExtractZip, vec![zip_dest.clone()])
            .with_destination(extract_dir.clone()),
    );
    let prog_unzip = manager.wait_for(id_unzip, TIMEOUT).expect("esperar extracción ZIP");
    assert_eq!(prog_unzip.state, OpState::Done, "La descompresión ZIP debe completarse");

    // 7. ENVIAR A LA PAPELERA
    let id_trash = manager.submit(
        OpRequest::new(OpKind::Trash, vec![renombrado_path.clone()]),
    );
    let prog_trash = manager.wait_for(id_trash, TIMEOUT).expect("esperar envío a papelera");
    assert!(prog_trash.state.is_finished(), "La operación de papelera debe finalizar");
    if prog_trash.state == OpState::Done {
        assert!(!renombrado_path.exists(), "El elemento enviado a la papelera ya no debe estar en su ruta original");
    }

    // 8. ELIMINAR PERMANENTEMENTE
    let id_eliminar = manager.submit(
        OpRequest::new(OpKind::DeletePermanently, vec![extract_dir.clone(), zip_dest.clone()]),
    );
    let prog_eliminar = manager.wait_for(id_eliminar, TIMEOUT).expect("esperar eliminación permanente");
    assert_eq!(prog_eliminar.state, OpState::Done, "La eliminación permanente debe completarse");
    assert!(!extract_dir.exists(), "El directorio extraído debe haberse eliminado permanentemente");
    assert!(!zip_dest.exists(), "El archivo ZIP debe haberse eliminado permanentemente");
}
