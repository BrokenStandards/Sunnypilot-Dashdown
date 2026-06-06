//! Per-segment file classification.

/// The kind of a file inside a segment directory. sunnypilot writes `.zst`
/// logs (legacy `.bz2`), HEVC camera streams, a `qcamera.ts` preview, and a
/// transient `rlog.lock` marker while the segment is still recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FileKind {
    FCamera,    // fcamera.hevc — road camera
    ECamera,    // ecamera.hevc — wide road camera
    DCamera,    // dcamera.hevc — driver camera (only if RecordFront)
    QCamera,    // qcamera.ts — low-res preview
    RLog,       // rlog.zst / rlog.bz2
    QLog,       // qlog.zst / qlog.bz2
    BootLog,    // bootlog.zst / bootlog.bz2
    LockMarker, // rlog.lock — segment is recording; never a download target
    Other,
}

impl FileKind {
    pub fn from_filename(name: &str) -> FileKind {
        match name {
            "fcamera.hevc" => FileKind::FCamera,
            "ecamera.hevc" => FileKind::ECamera,
            "dcamera.hevc" => FileKind::DCamera,
            "qcamera.ts" => FileKind::QCamera,
            "rlog.zst" | "rlog.bz2" => FileKind::RLog,
            "qlog.zst" | "qlog.bz2" => FileKind::QLog,
            "bootlog.zst" | "bootlog.bz2" => FileKind::BootLog,
            "rlog.lock" => FileKind::LockMarker,
            _ => FileKind::Other,
        }
    }

    /// A real artifact we may download (everything except the lock marker).
    pub fn is_downloadable(self) -> bool {
        !matches!(self, FileKind::LockMarker)
    }

    /// Part of the lightweight "previews only" selection.
    pub fn is_preview(self) -> bool {
        matches!(self, FileKind::QCamera)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::FCamera => "fcamera",
            FileKind::ECamera => "ecamera",
            FileKind::DCamera => "dcamera",
            FileKind::QCamera => "qcamera",
            FileKind::RLog => "rlog",
            FileKind::QLog => "qlog",
            FileKind::BootLog => "bootlog",
            FileKind::LockMarker => "lock",
            FileKind::Other => "other",
        }
    }

    /// Deserialize from the DB text written by [`FileKind::as_str`].
    pub fn from_db(s: &str) -> FileKind {
        match s {
            "fcamera" => FileKind::FCamera,
            "ecamera" => FileKind::ECamera,
            "dcamera" => FileKind::DCamera,
            "qcamera" => FileKind::QCamera,
            "rlog" => FileKind::RLog,
            "qlog" => FileKind::QLog,
            "bootlog" => FileKind::BootLog,
            "lock" => FileKind::LockMarker,
            _ => FileKind::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_files() {
        assert_eq!(FileKind::from_filename("fcamera.hevc"), FileKind::FCamera);
        assert_eq!(FileKind::from_filename("ecamera.hevc"), FileKind::ECamera);
        assert_eq!(FileKind::from_filename("dcamera.hevc"), FileKind::DCamera);
        assert_eq!(FileKind::from_filename("qcamera.ts"), FileKind::QCamera);
        assert_eq!(FileKind::from_filename("rlog.zst"), FileKind::RLog);
        assert_eq!(FileKind::from_filename("rlog.bz2"), FileKind::RLog);
        assert_eq!(FileKind::from_filename("qlog.zst"), FileKind::QLog);
        assert_eq!(FileKind::from_filename("qlog.bz2"), FileKind::QLog);
        assert_eq!(FileKind::from_filename("rlog.lock"), FileKind::LockMarker);
        assert_eq!(FileKind::from_filename("whatever.bin"), FileKind::Other);
    }

    #[test]
    fn lock_is_not_downloadable() {
        assert!(!FileKind::LockMarker.is_downloadable());
        assert!(FileKind::QCamera.is_downloadable());
    }

    #[test]
    fn previews_only_is_qcamera() {
        assert!(FileKind::QCamera.is_preview());
        assert!(!FileKind::FCamera.is_preview());
    }

    #[test]
    fn str_round_trip() {
        for k in [
            FileKind::FCamera,
            FileKind::ECamera,
            FileKind::DCamera,
            FileKind::QCamera,
            FileKind::RLog,
            FileKind::QLog,
            FileKind::BootLog,
            FileKind::LockMarker,
            FileKind::Other,
        ] {
            assert_eq!(FileKind::from_db(k.as_str()), k);
        }
    }
}
