use anyhow::{Result, bail};
use ratatui::layout::Rect;
use rebecca_core::config::AppRuntimeConfig;

use crate::runtime::CliRuntime;
use crate::tui::app::TuiApp;
use crate::tui::effect::TuiEffect;
use crate::tui::input::TuiMouseAction;
use crate::tui::layout;
use crate::tui::model::TuiScreen;
use crate::tui::terminal;

const REPLAY_HEIGHT: u16 = 30;

pub(crate) fn drive(
    app: &mut TuiApp,
    script: &str,
    terminal_width: usize,
    runtime_config: &AppRuntimeConfig,
    runtime: &CliRuntime,
) -> Result<()> {
    for token in script.split_whitespace() {
        let effect = effect_for_token(app, token, terminal_width)?;
        super::handle_effect(app, effect, runtime_config, runtime)?;
        if app.should_quit() {
            break;
        }
    }
    Ok(())
}

fn effect_for_token(app: &mut TuiApp, token: &str, terminal_width: usize) -> Result<TuiEffect> {
    if let Some(action) = mouse_action_for_token(app, token, terminal_width)? {
        return Ok(app.handle_mouse_action(action));
    }

    let Some(key) = terminal::replay_token_to_key(token) else {
        bail!("unknown tui replay key token: {token}");
    };
    Ok(app.handle_key(key))
}

fn mouse_action_for_token(
    app: &TuiApp,
    token: &str,
    terminal_width: usize,
) -> Result<Option<TuiMouseAction>> {
    if let Some(screen_token) = token.strip_prefix("click:tab:") {
        return Ok(Some(TuiMouseAction::SwitchScreen(screen_for_token(
            screen_token,
        )?)));
    }

    if let Some(index_token) = token.strip_prefix("click:row:") {
        return Ok(Some(row_action(app, parse_index(index_token, token)?)?));
    }

    if let Some(index_token) = token.strip_prefix("click:tile:") {
        return Ok(Some(tile_action(
            app,
            parse_index(index_token, token)?,
            terminal_width,
            false,
        )?));
    }

    if let Some(index_token) = token.strip_prefix("open:tile:") {
        return Ok(Some(tile_action(
            app,
            parse_index(index_token, token)?,
            terminal_width,
            true,
        )?));
    }

    match token {
        "wheel:up" | "scroll:up" => Ok(Some(TuiMouseAction::ScrollUp)),
        "wheel:down" | "scroll:down" => Ok(Some(TuiMouseAction::ScrollDown)),
        _ => Ok(None),
    }
}

fn screen_for_token(token: &str) -> Result<TuiScreen> {
    match token {
        "map" | "1" => Ok(TuiScreen::Map),
        "treemap" | "tree" | "4" => Ok(TuiScreen::Treemap),
        "types" | "type" | "2" => Ok(TuiScreen::Types),
        "extensions" | "extension" | "ext" | "3" => Ok(TuiScreen::Extensions),
        _ => bail!("unknown tui replay tab target: {token}"),
    }
}

fn row_action(app: &TuiApp, index: usize) -> Result<TuiMouseAction> {
    match app.screen {
        TuiScreen::Map | TuiScreen::Treemap => Ok(TuiMouseAction::SelectMapRow(index)),
        TuiScreen::Types | TuiScreen::Extensions => {
            Ok(TuiMouseAction::SelectDistributionRow(index))
        }
        _ => bail!("click:row is not available on {:?}", app.screen),
    }
}

fn tile_action(
    app: &TuiApp,
    index: usize,
    terminal_width: usize,
    open: bool,
) -> Result<TuiMouseAction> {
    if app.screen != TuiScreen::Treemap {
        bail!("tile replay actions are only available on the treemap screen");
    }

    let projection = app.frame_projection();
    let tile_area = replay_treemap_area(terminal_width);
    let tiles = layout::treemap_tiles(projection.visible_rows(), tile_area);
    let Some(tile) = tiles.get(index) else {
        bail!("tui replay tile index {index} is outside the visible treemap");
    };
    let Some(row_index) = tile.row_index else {
        return if open {
            Ok(TuiMouseAction::OpenTreemapAggregate)
        } else {
            bail!("tui replay tile index {index} points to a synthetic treemap bucket")
        };
    };
    Ok(if open {
        TuiMouseAction::OpenTreemapRow(row_index)
    } else {
        TuiMouseAction::SelectMapRow(row_index)
    })
}

fn replay_treemap_area(terminal_width: usize) -> Rect {
    let width = terminal_width.clamp(1, u16::MAX as usize) as u16;
    let frame = layout::frame(Rect::new(0, 0, width, REPLAY_HEIGHT));
    let workbench = layout::workbench_body(frame.body);
    layout::bordered_inner(workbench.primary)
}

fn parse_index(value: &str, token: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("invalid tui replay index in token: {token}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::input::TuiKey;

    #[test]
    fn screen_tokens_cover_visible_tabs() {
        assert_eq!(screen_for_token("map").unwrap(), TuiScreen::Map);
        assert_eq!(screen_for_token("treemap").unwrap(), TuiScreen::Treemap);
        assert_eq!(screen_for_token("types").unwrap(), TuiScreen::Types);
        assert_eq!(
            screen_for_token("extensions").unwrap(),
            TuiScreen::Extensions
        );
    }

    #[test]
    fn replay_treemap_area_keeps_clickable_body_inside_frame() {
        let area = replay_treemap_area(100);

        assert!(area.width > 0);
        assert!(area.height > 0);
        assert!(area.x > 0);
        assert!(area.y > 0);
    }

    #[test]
    fn invalid_index_reports_original_token() {
        let err = parse_index("x", "click:row:x").unwrap_err().to_string();

        assert!(err.contains("click:row:x"));
    }

    #[test]
    fn key_tokens_still_parse_through_terminal_mapping() {
        assert_eq!(terminal::replay_token_to_key("j"), Some(TuiKey::Down));
        assert_eq!(terminal::replay_token_to_key("4"), Some(TuiKey::Char('4')));
    }

    #[test]
    fn tile_open_token_uses_treemap_open_action() {
        let mut app = TuiApp::root_picker(
            Vec::new(),
            rebecca_core::scan::ScanBackendKind::PortableRecursive,
            10,
        );
        app.screen = TuiScreen::Treemap;

        let err = mouse_action_for_token(&app, "open:tile:0", 80)
            .unwrap_err()
            .to_string();

        assert!(err.contains("outside the visible treemap"));
    }
}
