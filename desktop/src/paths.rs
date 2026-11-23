use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    let dir = if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
        PathBuf::from(d).join("sala-de-tela")
    } else if let Some(d) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(d).join("SalaDeTela")
    } else {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join(".local/share/sala-de-tela")
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}
