use super::ProjectArtifactDefinition;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectArtifactContext {
    NodeProject,
    TargetProject,
    PythonProject,
    PythonCache,
    GradleProject,
    DartProject,
    ZigProject,
    GenericProjectOutput,
    CocoapodsProject,
    CxxProject,
    ExpoProject,
    SwiftPackage,
    DotnetBin,
    DotnetObj,
    ComposerVendor,
    CachedirTag,
}

pub const CACHEDIR_TAG_DEFINITION: ProjectArtifactDefinition = ProjectArtifactDefinition {
    directory_name: "CACHEDIR.TAG",
    rule_id: "windows.project-artifact-cachedir-tag",
    restore_hint: "CACHEDIR.TAG marks this directory as rebuildable cache data.",
};
