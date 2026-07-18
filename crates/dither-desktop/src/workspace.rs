use std::{
    fs,
    path::{Path, PathBuf},
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
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(temporary, path).map_err(|error| error.to_string())
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

        let mut state = PersistentState::default();
        state.remember_file("one.tif".into());
        state.remember_file("two.tif".into());
        state.remember_file("one.tif".into());
        assert_eq!(
            state.recent_files,
            [PathBuf::from("one.tif"), PathBuf::from("two.tif")]
        );
    }
}
