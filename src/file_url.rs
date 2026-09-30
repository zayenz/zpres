use std::path::{Path, PathBuf};

pub(crate) fn file_url(path: &Path) -> String {
    let path = absolute_path(path);
    file_url_from_absolute_path(&path)
}

#[cfg(unix)]
fn file_url_from_absolute_path(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let bytes = path.as_os_str().as_bytes();
    let mut encoded = String::from("file://");
    if !bytes.starts_with(b"/") {
        encoded.push('/');
    }
    push_percent_encoded_path(&mut encoded, bytes);
    encoded
}

#[cfg(windows)]
fn file_url_from_absolute_path(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let normalized = if let Some(rest) = normalized.strip_prefix("//?/UNC/") {
        format!("//{rest}")
    } else if let Some(rest) = normalized.strip_prefix("//?/") {
        rest.to_string()
    } else {
        normalized
    };

    if let Some(unc) = normalized.strip_prefix("//") {
        let (host, path) = unc.split_once('/').unwrap_or((unc, ""));
        let mut encoded = String::from("file://");
        push_percent_encoded_host(&mut encoded, host.as_bytes());
        if !path.is_empty() {
            encoded.push('/');
            push_percent_encoded_path(&mut encoded, path.as_bytes());
        }
        encoded
    } else {
        let mut encoded = String::from("file:///");
        push_percent_encoded_path(&mut encoded, normalized.trim_start_matches('/').as_bytes());
        encoded
    }
}

#[cfg(not(any(unix, windows)))]
fn file_url_from_absolute_path(path: &Path) -> String {
    let bytes = path.to_string_lossy();
    let mut encoded = String::from("file://");
    if !bytes.starts_with('/') {
        encoded.push('/');
    }
    push_percent_encoded_path(&mut encoded, bytes.as_bytes());
    encoded
}

fn push_percent_encoded_path(encoded: &mut String, bytes: &[u8]) {
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
}

#[cfg(windows)]
fn push_percent_encoded_host(encoded: &mut String, bytes: &[u8]) {
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
}

fn absolute_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|current_dir| current_dir.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn percent_encodes_reserved_and_non_ascii_path_bytes() {
        assert_eq!(
            file_url(Path::new("/tmp/a visual # % ü/page.html")),
            "file:///tmp/a%20visual%20%23%20%25%20%C3%BC/page.html"
        );
    }

    #[test]
    fn makes_relative_paths_absolute() {
        let value = file_url(Path::new("visual/page.html"));

        assert!(value.starts_with("file:///"));
        assert!(value.ends_with("/visual/page.html"));
    }

    #[cfg(unix)]
    #[test]
    fn percent_encodes_a_unix_backslash_instead_of_treating_it_as_a_separator() {
        assert_eq!(
            file_url(Path::new("/tmp/a\\b.svg")),
            "file:///tmp/a%5Cb.svg"
        );
    }

    #[cfg(unix)]
    #[test]
    fn percent_encodes_non_utf8_unix_path_bytes_without_replacement() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(OsString::from_vec(b"/tmp/non-\xFF/path.svg".to_vec()));

        assert_eq!(file_url(&path), "file:///tmp/non-%FF/path.svg");
    }

    #[cfg(windows)]
    #[test]
    fn renders_windows_drive_paths_as_file_urls() {
        assert_eq!(
            file_url_from_absolute_path(Path::new(r"C:\Talks\a # %.html")),
            "file:///C:/Talks/a%20%23%20%25.html"
        );
    }

    #[cfg(windows)]
    #[test]
    fn renders_windows_unc_paths_with_the_server_as_host() {
        assert_eq!(
            file_url_from_absolute_path(Path::new(r"\\server\share\a # %.html")),
            "file://server/share/a%20%23%20%25.html"
        );
    }
}
