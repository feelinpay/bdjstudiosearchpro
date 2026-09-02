pub mod arena;
pub mod builder;
pub mod ext_table;
pub mod layout;
pub mod mmap;
pub mod overlay;
pub mod overlay_file;
pub mod settings;
pub mod suppress;
pub mod view;
pub mod vol_table;

pub use arena::NameArena;
pub use builder::IndexBuilder;
pub use ext_table::InternedExtensions;
pub use layout::{Header, SectionId, SectionTable, CURRENT_VERSION, MAGIC};
pub use mmap::MmapIndex;
pub use overlay::OverlayIndex;
pub use overlay_file::{
    OVERLAY_FILE_NAME, OverlayFileError, OverlaySnapshot, overlay_path_for, remove_overlay,
    write_overlay,
};
pub use settings::IndexerSettings;
pub use suppress::{SuppressionWindow, normalize_path};
pub use view::{IndexView, ViewError};
pub use vol_table::{VolumeRecord, VolumeTable};
