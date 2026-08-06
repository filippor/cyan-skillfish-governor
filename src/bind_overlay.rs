use log::trace;
use std::{
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Seek, SeekFrom, Write},
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};

const FILE_PERMS: u32 = 0o644;

pub struct BindOverlay {
    file: File,
    target: String,
}

impl BindOverlay {
    pub fn create(temp_path: &str, target: &str, initial_content: &[u8]) -> io::Result<Self> {
        trace!("Unmounting stale bind: {target}");
        let _ = umount_bind(target);

        trace!("Creating bind overlay: {temp_path}");
        let _ = fs::remove_file(temp_path);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(temp_path)?;
        fs::set_permissions(temp_path, Permissions::from_mode(FILE_PERMS))?;
        file.write_all(initial_content)?;
        file.flush()?;

        trace!("Binding {temp_path} to {target}");
        mount_bind(temp_path, target)?;
        fs::remove_file(temp_path)?;

        Ok(Self {
            file,
            target: target.to_owned(),
        })
    }

    pub fn replace(&mut self, content: &[u8]) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.set_len(0)?;
        self.file.write_all(content)?;
        self.file.flush()
    }

    pub fn shutdown(&self) -> io::Result<()> {
        umount_bind(&self.target)
    }
}

fn mount_bind(src: &str, dst: &str) -> io::Result<()> {
    let status = Command::new("mount").args(["--bind", src, dst]).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "mount --bind {} {} failed: {}",
            src, dst, status
        )))
    }
}

fn umount_bind(dst: &str) -> io::Result<()> {
    let status = Command::new("umount")
        .arg(dst)
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "umount {} failed: {}",
            dst, status
        )))
    }
}
