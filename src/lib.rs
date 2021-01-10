// Copyright 2020 Sebastian Wiesner <sebastian@swsnr.de>

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Install binaries to $HOME.
//!
//! Not a package manager.

#![deny(warnings, clippy::all, missing_docs)]

use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, Context, Error};
use colored::Colorize;
use fehler::throws;
use versions::Versioning;

pub use dirs::*;
pub use manifest::{Manifest, ManifestRepo, ManifestStore};
pub use repos::HomebinRepos;

use crate::operations::{ApplyOperation, RemoveOperation};
use crate::tools::{manpath, path_contains};

mod checksum;
mod dirs;
mod process;
mod repos;
mod tools;

/// Manifest types and loading.
pub mod manifest;
/// Operations to apply manifests to a home directory.
pub mod operations;

/// Check whether the environment is ok, and print warnings to stderr if not.
///
/// This specifically checks whether `install_dirs` are contained in the relevant environment variables
/// such as `$PATH` or `$MANPATH`.
#[throws]
pub fn check_environment(install_dirs: &InstallDirs) -> () {
    match std::env::var_os("PATH") {
        None => eprintln!("{}", "WARNING: $PATH not set!".yellow().bold()),
        Some(path) => {
            if !path_contains(&path, install_dirs.bin_dir()) {
                eprintln!(
                    "{}\nAdd {} to $PATH in your shell profile.",
                    format!(
                        "WARNING: $PATH does not contain bin dir at {}",
                        install_dirs.bin_dir().display()
                    )
                    .yellow()
                    .bold(),
                    install_dirs.bin_dir().display()
                )
            }
        }
    };

    if !path_contains(&manpath()?, install_dirs.man_dir()) {
        eprintln!(
            "{}\nAdd {} to $MANPATH in your shell profile; see man 1 manpath for more information",
            format!(
                "WARNING: manpath does not contain man dir at {}",
                install_dirs.man_dir().display()
            )
            .yellow()
            .bold(),
            install_dirs.man_dir().display()
        );
    }
}

/// Install a manifest.
///
/// Apply the operations of a `manifest` against the given `install_dirs`; using the given project `dirs` for downloads.
#[throws]
pub fn install_manifest(
    dirs: &HomebinProjectDirs,
    install_dirs: &mut InstallDirs,
    manifest: &Manifest,
) -> () {
    let op_dirs = ManifestOperationDirs::for_manifest(dirs, install_dirs, manifest)?;
    let operations = operations::install_manifest(manifest);
    std::fs::create_dir_all(op_dirs.download_dir()).with_context(|| {
        format!(
            "Failed to create download directory at {}",
            dirs.download_dir().display()
        )
    })?;

    for operation in operations {
        operation.apply_operation(&op_dirs)?;
    }
}

/// Remove a manifest.
///
/// Apply the remove operations of the `manifest` aganst the given install dirs.
#[throws]
pub fn remove_manifest(
    dirs: &HomebinProjectDirs,
    install_dirs: &mut InstallDirs,
    manifest: &Manifest,
) -> () {
    let op_dirs = ManifestOperationDirs::for_manifest(dirs, install_dirs, manifest)?;
    let operations = operations::remove_manifest(manifest);
    for operation in operations {
        operation.apply_operation(&op_dirs)?;
    }
}

/// Get the installed version of the given manifest.
///
/// Attempt to invoke the version check denoted in the manifest, i.e. the given binary with the
/// version check arguments, and use the pattern to extract a version number.
///
/// Return `None` if the binary doesn't exist or its output doesn't match the pattern;
/// fail if we cannot invoke it for other reasons or if we fail to parse the version from other.
#[throws]
pub fn installed_manifest_version(dirs: &InstallDirs, manifest: &Manifest) -> Option<Versioning> {
    let args = &manifest.discover.version_check.args;
    let binary = dirs.bin_dir().join(&manifest.discover.binary);
    if binary.is_file() {
        let output = Command::new(&binary).args(args).output().with_context(|| {
            format!(
                "Failed to run {} with {:?}",
                binary.display(),
                &manifest.discover.version_check.args
            )
        })?;
        let pattern = manifest.discover.version_check.regex().with_context(|| {
            format!(
                "Version check for {} failed: Invalid regex {}",
                manifest.info.name, manifest.discover.version_check.pattern
            )
        })?;
        let output = std::str::from_utf8(&output.stdout).with_context(|| {
            format!(
                "Output of command {} with {:?} returned non-utf8 stdout: {:?}",
                binary.display(),
                args,
                output.stdout
            )
        })?;
        let version = pattern
            .captures(output)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str());

        version
            .map(|s| {
                Versioning::new(s).ok_or_else(|| {
                    anyhow!(
                        "Output of command {} with {:?} returned invalid version {:?}",
                        binary.display(),
                        args,
                        version
                    )
                })
            })
            .transpose()?
    } else {
        None
    }
}

/// Whether the given manifest is outdated and needs updating.
///
/// Return the installed version if it's outdated, otherwise return None.
#[throws]
pub fn outdated_manifest_version(dirs: &InstallDirs, manifest: &Manifest) -> Option<Versioning> {
    installed_manifest_version(dirs, manifest)?
        .filter(|installed| installed < &manifest.info.version)
}

/// Get all files the `manifest` would install to `dirs`.
pub fn installed_files(dirs: &InstallDirs, manifest: &Manifest) -> Vec<PathBuf> {
    operations::operation_destinations(operations::install_manifest(manifest).iter())
        .map(|destination| dirs.path(destination.directory()).join(destination.name()))
        .collect()
}

/// Get all files that would be removed when removing `manifest`.
pub fn files_to_remove(dirs: &InstallDirs, manifest: &Manifest) -> Vec<PathBuf> {
    operations::remove_manifest(manifest)
        .iter()
        .map(|op| match op {
            RemoveOperation::Delete(dir, name) => dirs.path(*dir).join(name.as_ref()),
        })
        .collect()
}
