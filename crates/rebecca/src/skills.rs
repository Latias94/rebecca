use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::cli::{OutputMode, SkillAgentArg, SkillsInstallArgs, SkillsPathArgs, SkillsRemoveArgs};
use crate::output::CliApiContract;

const SKILL_NAME: &str = "rebecca-disk-cleaner";
const SKILL_FILE_NAME: &str = "SKILL.md";
const MANIFEST_FILE_NAME: &str = ".rebecca-skill.json";
const PACKAGED_SKILL: &str = include_str!("../skills/rebecca-disk-cleaner/SKILL.md");

#[derive(Debug, Serialize)]
struct SkillManagementReport {
    action: &'static str,
    status: &'static str,
    skill: &'static str,
    agent: &'static str,
    skills_dir: PathBuf,
    skill_dir: PathBuf,
    dry_run: bool,
    changed: bool,
    managed: bool,
    message: String,
}

#[derive(Debug, Serialize)]
struct SkillInstallManifest {
    package: &'static str,
    skill: &'static str,
    version: &'static str,
    managed_by: &'static str,
}

#[derive(Debug)]
struct SkillTarget {
    agent: &'static str,
    skills_dir: PathBuf,
    skill_dir: PathBuf,
}

pub(crate) fn install(output_mode: OutputMode, args: SkillsInstallArgs) -> Result<()> {
    let target = resolve_target(args.target.agent, args.target.destination)?;
    let existing = inspect_existing_skill(&target.skill_dir)?;
    let report = if existing.exists && existing.same_content {
        SkillManagementReport {
            action: "install",
            status: "unchanged",
            skill: SKILL_NAME,
            agent: target.agent,
            skills_dir: target.skills_dir,
            skill_dir: target.skill_dir,
            dry_run: args.dry_run,
            changed: false,
            managed: existing.managed,
            message: "Rebecca skill is already installed with the packaged content.".to_string(),
        }
    } else if existing.exists && !args.force {
        bail!(
            "Rebecca skill already exists at {}; pass --force to replace it",
            target.skill_dir.display()
        );
    } else if args.dry_run {
        let status = if existing.exists {
            "would-replace"
        } else {
            "would-install"
        };
        let message = install_message(status, &target.skill_dir);
        SkillManagementReport {
            action: "install",
            status,
            skill: SKILL_NAME,
            agent: target.agent,
            skills_dir: target.skills_dir,
            skill_dir: target.skill_dir,
            dry_run: true,
            changed: false,
            managed: existing.managed,
            message,
        }
    } else {
        write_skill_directory(&target)?;
        SkillManagementReport {
            action: "install",
            status: "installed",
            skill: SKILL_NAME,
            agent: target.agent,
            skills_dir: target.skills_dir,
            skill_dir: target.skill_dir,
            dry_run: false,
            changed: true,
            managed: true,
            message: "Rebecca skill installed. Restart the agent to load it.".to_string(),
        }
    };

    print_report(output_mode, "skills install", &report)
}

pub(crate) fn remove(output_mode: OutputMode, args: SkillsRemoveArgs) -> Result<()> {
    let target = resolve_target(args.target.agent, args.target.destination)?;
    let existing = inspect_existing_skill(&target.skill_dir)?;
    let report = if !existing.exists {
        SkillManagementReport {
            action: "remove",
            status: "missing",
            skill: SKILL_NAME,
            agent: target.agent,
            skills_dir: target.skills_dir,
            skill_dir: target.skill_dir,
            dry_run: args.dry_run,
            changed: false,
            managed: false,
            message: "Rebecca skill is not installed at the selected path.".to_string(),
        }
    } else if !existing.managed && !args.force {
        bail!(
            "refusing to remove {}; it does not look like a Rebecca-managed skill. Pass --force only if this is the intended skill directory",
            target.skill_dir.display()
        );
    } else if args.dry_run {
        let message = format!(
            "Dry run only; Rebecca would remove {}.",
            target.skill_dir.display()
        );
        SkillManagementReport {
            action: "remove",
            status: "would-remove",
            skill: SKILL_NAME,
            agent: target.agent,
            skills_dir: target.skills_dir,
            skill_dir: target.skill_dir,
            dry_run: true,
            changed: false,
            managed: existing.managed,
            message,
        }
    } else {
        fs::remove_dir_all(&target.skill_dir)
            .with_context(|| format!("failed to remove {}", target.skill_dir.display()))?;
        SkillManagementReport {
            action: "remove",
            status: "removed",
            skill: SKILL_NAME,
            agent: target.agent,
            skills_dir: target.skills_dir,
            skill_dir: target.skill_dir,
            dry_run: false,
            changed: true,
            managed: existing.managed,
            message: "Rebecca skill removed. Restart the agent if it was already running."
                .to_string(),
        }
    };

    print_report(output_mode, "skills remove", &report)
}

pub(crate) fn path(output_mode: OutputMode, args: SkillsPathArgs) -> Result<()> {
    let target = resolve_target(args.target.agent, args.target.destination)?;
    let existing = inspect_existing_skill(&target.skill_dir)?;
    let report = SkillManagementReport {
        action: "path",
        status: "path",
        skill: SKILL_NAME,
        agent: target.agent,
        skills_dir: target.skills_dir,
        skill_dir: target.skill_dir,
        dry_run: false,
        changed: false,
        managed: existing.managed,
        message: if existing.exists {
            "Rebecca skill path is installed.".to_string()
        } else {
            "Rebecca skill path is not installed yet.".to_string()
        },
    };

    print_report(output_mode, "skills path", &report)
}

fn print_report(
    output_mode: OutputMode,
    command: &'static str,
    report: &SkillManagementReport,
) -> Result<()> {
    crate::output::print_command_success_with_contract(
        CliApiContract::v1(command, "skill-management"),
        output_mode,
        || report,
        || {
            println!("{}", report.message);
            println!("Skill: {}", report.skill);
            println!("Agent: {}", report.agent);
            println!("Skills dir: {}", report.skills_dir.display());
            println!("Skill dir: {}", report.skill_dir.display());
            println!("Status: {}", report.status);
            Ok(())
        },
    )
}

fn resolve_target(agent: SkillAgentArg, destination: Option<PathBuf>) -> Result<SkillTarget> {
    let agent_label = if destination.is_some() {
        "custom"
    } else {
        agent.label()
    };
    let skills_dir = match destination {
        Some(path) => expand_home(path)?,
        None => default_skills_dir(agent)?,
    };
    reject_skill_dir_as_destination(&skills_dir)?;
    let skill_dir = skills_dir.join(SKILL_NAME);

    Ok(SkillTarget {
        agent: agent_label,
        skills_dir,
        skill_dir,
    })
}

fn default_skills_dir(agent: SkillAgentArg) -> Result<PathBuf> {
    let home = home_dir()?;
    Ok(match agent {
        SkillAgentArg::Agents => home.join(".agents").join("skills"),
        SkillAgentArg::Codex => env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"))
            .join("skills"),
    })
}

fn home_dir() -> Result<PathBuf> {
    if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    if let Some(profile) = env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(profile));
    }
    if let (Some(drive), Some(path)) = (env::var_os("HOMEDRIVE"), env::var_os("HOMEPATH"))
        && !drive.is_empty()
        && !path.is_empty()
    {
        return Ok(PathBuf::from(format!(
            "{}{}",
            drive.to_string_lossy(),
            path.to_string_lossy()
        )));
    }

    bail!("could not resolve the current user's home directory")
}

fn expand_home(path: PathBuf) -> Result<PathBuf> {
    let raw = path.as_os_str().to_string_lossy();
    if raw == "~" {
        return home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return Ok(home_dir()?.join(rest));
    }
    Ok(path)
}

fn reject_skill_dir_as_destination(skills_dir: &Path) -> Result<()> {
    if skills_dir.file_name().and_then(|name| name.to_str()) == Some(SKILL_NAME) {
        bail!(
            "--destination expects the parent skills directory, not the {} skill directory",
            SKILL_NAME
        );
    }
    Ok(())
}

#[derive(Debug)]
struct ExistingSkill {
    exists: bool,
    managed: bool,
    same_content: bool,
}

fn inspect_existing_skill(skill_dir: &Path) -> Result<ExistingSkill> {
    if !skill_dir.exists() {
        return Ok(ExistingSkill {
            exists: false,
            managed: false,
            same_content: false,
        });
    }
    if !skill_dir.is_dir() {
        bail!(
            "Rebecca skill path exists but is not a directory: {}",
            skill_dir.display()
        );
    }

    let skill_file = skill_dir.join(SKILL_FILE_NAME);
    let content = fs::read_to_string(&skill_file).with_context(|| {
        format!(
            "existing skill directory is missing readable {}: {}",
            SKILL_FILE_NAME,
            skill_file.display()
        )
    })?;
    let has_manifest = skill_dir.join(MANIFEST_FILE_NAME).is_file();
    let has_rebecca_frontmatter = content
        .lines()
        .take(12)
        .any(|line| line.trim() == format!("name: {SKILL_NAME}"));

    Ok(ExistingSkill {
        exists: true,
        managed: has_manifest || has_rebecca_frontmatter,
        same_content: content == PACKAGED_SKILL,
    })
}

fn write_skill_directory(target: &SkillTarget) -> Result<()> {
    fs::create_dir_all(&target.skills_dir)
        .with_context(|| format!("failed to create {}", target.skills_dir.display()))?;

    let temp_dir = target
        .skills_dir
        .join(format!(".{SKILL_NAME}.tmp-{}", std::process::id()));
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)
            .with_context(|| format!("failed to clear stale {}", temp_dir.display()))?;
    }

    fs::create_dir(&temp_dir)
        .with_context(|| format!("failed to create temporary {}", temp_dir.display()))?;
    fs::write(temp_dir.join(SKILL_FILE_NAME), PACKAGED_SKILL).with_context(|| {
        format!(
            "failed to write {}",
            temp_dir.join(SKILL_FILE_NAME).display()
        )
    })?;
    fs::write(
        temp_dir.join(MANIFEST_FILE_NAME),
        serde_json::to_vec_pretty(&SkillInstallManifest {
            package: "rebecca",
            skill: SKILL_NAME,
            version: env!("CARGO_PKG_VERSION"),
            managed_by: "rebecca skills install",
        })?,
    )
    .with_context(|| {
        format!(
            "failed to write {}",
            temp_dir.join(MANIFEST_FILE_NAME).display()
        )
    })?;

    if target.skill_dir.exists() {
        fs::remove_dir_all(&target.skill_dir)
            .with_context(|| format!("failed to replace {}", target.skill_dir.display()))?;
    }
    fs::rename(&temp_dir, &target.skill_dir).with_context(|| {
        format!(
            "failed to move {} to {}",
            temp_dir.display(),
            target.skill_dir.display()
        )
    })?;

    Ok(())
}

fn install_message(status: &str, skill_dir: &Path) -> String {
    match status {
        "would-replace" => format!(
            "Dry run only; Rebecca would replace the existing skill at {}.",
            skill_dir.display()
        ),
        _ => format!(
            "Dry run only; Rebecca would install the skill at {}.",
            skill_dir.display()
        ),
    }
}
