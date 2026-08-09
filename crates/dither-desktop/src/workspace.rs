use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use dither_core::Recipe;
use dither_io::ExportFormat;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrowserSort {
    #[default]
    Name,
    Modified,
    FileType,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SavedExportFormat {
    Png16,
    #[default]
    Tiff16,
    OpenExr32,
}

impl From<SavedExportFormat> for ExportFormat {
    fn from(value: SavedExportFormat) -> Self {
        match value {
            SavedExportFormat::Png16 => Self::Png16,
            SavedExportFormat::Tiff16 => Self::Tiff16,
            SavedExportFormat::OpenExr32 => Self::OpenExr32,
        }
    }
}

impl SavedExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png16 => "png",
            Self::Tiff16 => "tif",
            Self::OpenExr32 => "exr",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Png16 => "PNG 16-bit",
            Self::Tiff16 => "TIFF 16-bit",
            Self::OpenExr32 => "OpenEXR 32-bit",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    pub directory: Option<PathBuf>,
    pub naming: String,
    pub format: SavedExportFormat,
    pub plates: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            directory: None,
            naming: "{name}-dithered".into(),
            format: SavedExportFormat::Tiff16,
            plates: false,
        }
    }
}

impl ExportSettings {
    pub fn destination(&self, source: &Path) -> Option<PathBuf> {
        let directory = self.directory.as_deref().or_else(|| source.parent())?;
        let name = source
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("image");
        let filename = self.naming.replace("{name}", name);
        Some(
            directory
                .join(filename)
                .with_extension(self.format.extension()),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectFile {
    pub version: u32,
    pub source: PathBuf,
    pub recipe: Recipe,
    pub export: ExportSettings,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectState {
    pub path: Option<PathBuf>,
    clean: Option<ProjectFile>,
}

impl ProjectState {
    pub fn clean(path: Option<PathBuf>, project: ProjectFile) -> Self {
        Self {
            path,
            clean: Some(project),
        }
    }

    pub fn recovered(project_path: Option<PathBuf>) -> Self {
        Self {
            path: project_path,
            clean: None,
        }
    }

    pub fn is_dirty(&self, project: &ProjectFile) -> bool {
        self.clean.as_ref() != Some(project)
    }

    pub fn mark_saved(&mut self, path: PathBuf, project: ProjectFile) {
        self.path = Some(path);
        self.clean = Some(project);
    }

    pub fn discard_changes(&mut self, project: ProjectFile) {
        self.clean = Some(project);
    }
}

impl Default for ProjectFile {
    fn default() -> Self {
        Self {
            version: 1,
            source: PathBuf::new(),
            recipe: Recipe::default(),
            export: ExportSettings::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportRecord {
    pub source: PathBuf,
    pub outputs: Vec<PathBuf>,
    pub completed_unix_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistentState {
    pub recent_files: Vec<PathBuf>,
    pub favorite_folders: Vec<PathBuf>,
    pub last_folder: Option<PathBuf>,
    pub browser_sort: BrowserSort,
    pub export_history: Vec<ExportRecord>,
    pub default_export: ExportSettings,
}

impl Default for PersistentState {
    fn default() -> Self {
        Self {
            recent_files: Vec::new(),
            favorite_folders: Vec::new(),
            last_folder: None,
            browser_sort: BrowserSort::Name,
            export_history: Vec::new(),
            default_export: ExportSettings::default(),
        }
    }
}

impl PersistentState {
    pub fn remember_file(&mut self, path: PathBuf) {
        self.recent_files.retain(|recent| recent != &path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(20);
    }

    pub fn remember_export(&mut self, record: ExportRecord) {
        self.export_history.insert(0, record);
        self.export_history.truncate(50);
    }
}

pub fn save_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if path.exists() && !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let temporary = save_sidecar_path(path, "temporary");
    let backup = save_sidecar_path(path, "backup");
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    drop(file);

    let had_destination = path.exists();
    if had_destination && let Err(error) = fs::rename(path, &backup) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let restore = if had_destination {
            fs::rename(&backup, path)
        } else {
            Ok(())
        };
        let _ = fs::remove_file(&temporary);
        return match restore {
            Ok(()) => Err(error.to_string()),
            Err(restore) => Err(format!(
                "{error}; the previous file could not be restored: {restore}"
            )),
        };
    }
    if had_destination {
        let _ = fs::remove_file(backup);
    }
    if let Some(parent) = path.parent()
        && let Ok(directory) = fs::File::open(parent)
    {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn save_sidecar_path(path: &Path, kind: &str) -> PathBuf {
    static NEXT_SAVE: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let serial = NEXT_SAVE.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(
        ".{name}.dither-{kind}-{}-{serial}",
        std::process::id()
    ))
}

pub fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_and_state_round_trip_and_naming_is_stable() {
        let project = ProjectFile {
            source: "/images/source.tif".into(),
            export: ExportSettings {
                directory: Some("/exports".into()),
                naming: "print-{name}".into(),
                ..ExportSettings::default()
            },
            ..ProjectFile::default()
        };
        let bytes = serde_json::to_vec(&project).unwrap();
        assert_eq!(
            serde_json::from_slice::<ProjectFile>(&bytes).unwrap(),
            project
        );
        assert_eq!(
            project.export.destination(&project.source).unwrap(),
            PathBuf::from("/exports/print-source.tif")
        );
        assert_eq!(SavedExportFormat::Png16.label(), "PNG 16-bit");
        assert_eq!(SavedExportFormat::OpenExr32.label(), "OpenEXR 32-bit");

        let mut state = PersistentState::default();
        state.remember_file("one.tif".into());
        state.remember_file("two.tif".into());
        state.remember_file("one.tif".into());
        assert_eq!(
            state.recent_files,
            [PathBuf::from("one.tif"), PathBuf::from("two.tif")]
        );

        let mut project_state = ProjectState::clean(Some("project.dither".into()), project.clone());
        assert!(!project_state.is_dirty(&project));
        let mut edited = project.clone();
        edited.recipe.preprocess.brightness = 0.25;
        assert!(project_state.is_dirty(&edited));
        project_state.mark_saved("project.dither".into(), edited.clone());
        assert!(!project_state.is_dirty(&edited));
        assert_eq!(project_state.path, Some("project.dither".into()));
        assert!(ProjectState::recovered(None).is_dirty(&project));
    }

    #[test]
    fn json_save_replaces_the_complete_file_without_sidecars() {
        let directory =
            std::env::temp_dir().join(format!("dither-workspace-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("project.dither");
        let mut project = ProjectFile {
            source: "first.tif".into(),
            ..ProjectFile::default()
        };
        save_json(&path, &project).unwrap();
        project.source = "second.tif".into();
        save_json(&path, &project).unwrap();

        assert_eq!(load_json::<ProjectFile>(&path).unwrap(), project);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);

        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
