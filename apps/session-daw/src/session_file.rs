//! Saving a song: `Song.session`, its project file (`crate::project_file`)
//! — beside the `Song.RPP` it was prepared from, its media paths resolving
//! against that same folder.

/// Save `project_guid` to the project file `path` names (`Song.session`,
/// or an alternative `Song.session/Organized`): what changed since its
/// last save, as one step of its history. Says where.
///
/// # Errors
/// The file would not take the save, or the engine would not be read.
pub fn save_session(
    daw: &daw::standalone::Standalone,
    project_guid: &str,
    path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    #[cfg(feature = "native")]
    {
        let (file, branch) = crate::project_file::save_target(path);
        crate::project_file::save(daw, project_guid, &file, &branch, "Save")
    }
    #[cfg(not(feature = "native"))]
    {
        let _ = (daw, project_guid, path);
        Err("a page saves nothing".to_owned())
    }
}
