use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct MacFsChangeEvent {
    pub path: PathBuf,
    pub event_id: u64,
    pub is_created: bool,
    pub is_removed: bool,
    pub is_renamed: bool,
    pub is_modified: bool,
    pub is_dir: bool,
    pub must_rescan_subdirs: bool,
}

// Flags defined by Darwin FSEvents.h
pub const K_FSEVENT_STREAM_EVENT_FLAG_NONE: u32 = 0x00000000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_MUST_SCAN_SUBDIRS: u32 = 0x00000001;
pub const K_FSEVENT_STREAM_EVENT_FLAG_USER_DROPPED: u32 = 0x00000002;
pub const K_FSEVENT_STREAM_EVENT_FLAG_KERNEL_DROPPED: u32 = 0x00000004;
pub const K_FSEVENT_STREAM_EVENT_FLAG_EVENT_IDS_WRAPPED: u32 = 0x00000008;
pub const K_FSEVENT_STREAM_EVENT_FLAG_HISTORY_DONE: u32 = 0x00000010;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ROOT_CHANGED: u32 = 0x00000020;
pub const K_FSEVENT_STREAM_EVENT_FLAG_MOUNT: u32 = 0x00000040;
pub const K_FSEVENT_STREAM_EVENT_FLAG_UNMOUNT: u32 = 0x00000080;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_CREATED: u32 = 0x00000100;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_REMOVED: u32 = 0x00000200;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_INODE_META_MOD: u32 = 0x00000400;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_RENAMED: u32 = 0x00000800;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_MODIFIED: u32 = 0x00001000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_FINDER_INFO_MOD: u32 = 0x00002000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_CHANGE_OWNER: u32 = 0x00004000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_XATTR_MOD: u32 = 0x00008000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_IS_FILE: u32 = 0x00010000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_IS_DIR: u32 = 0x00020000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_IS_SYMLINK: u32 = 0x00040000;

pub struct FsEventStreamWatcher {
    pub watch_path: PathBuf,
    pub last_event_id: u64,
}

impl FsEventStreamWatcher {
    pub fn new(path: &Path, start_event_id: u64) -> Self {
        Self {
            watch_path: path.to_path_buf(),
            last_event_id: start_event_id,
        }
    }

    /// Decodes raw Darwin FSEvents bitmask flags into high-level event
    pub fn parse_event(path: PathBuf, event_id: u64, flags: u32) -> MacFsChangeEvent {
        let is_created = (flags & K_FSEVENT_STREAM_EVENT_FLAG_ITEM_CREATED) != 0;
        let is_removed = (flags & K_FSEVENT_STREAM_EVENT_FLAG_ITEM_REMOVED) != 0;
        let is_renamed = (flags & K_FSEVENT_STREAM_EVENT_FLAG_ITEM_RENAMED) != 0;
        let is_modified = (flags & K_FSEVENT_STREAM_EVENT_FLAG_ITEM_MODIFIED) != 0;
        let is_dir = (flags & K_FSEVENT_STREAM_EVENT_FLAG_ITEM_IS_DIR) != 0;
        let must_rescan = (flags & (K_FSEVENT_STREAM_EVENT_FLAG_MUST_SCAN_SUBDIRS
            | K_FSEVENT_STREAM_EVENT_FLAG_USER_DROPPED
            | K_FSEVENT_STREAM_EVENT_FLAG_KERNEL_DROPPED)) != 0;

        MacFsChangeEvent {
            path,
            event_id,
            is_created,
            is_removed,
            is_renamed,
            is_modified,
            is_dir,
            must_rescan_subdirs: must_rescan,
        }
    }
}
