use std::sync::Mutex;
use tauri::{LogicalSize, PhysicalPosition, PhysicalSize, State, WebviewWindow};

const PET_WIDTH: f64 = 240.0;
const PET_HEIGHT: f64 = 300.0;

struct MainWindow {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    maximized: bool,
}

#[derive(Default)]
struct Mode {
    main: Option<MainWindow>,
    pet_position: Option<PhysicalPosition<i32>>,
}

#[derive(Default)]
pub struct WindowMode(Mutex<Mode>);

fn restore(window: &WebviewWindow, saved: &MainWindow) -> tauri::Result<()> {
    window.set_always_on_top(false)?;
    window.set_resizable(true)?;
    window.set_decorations(true)?;
    window.set_shadow(true)?;
    window.set_min_size(Some(LogicalSize::new(680.0, 600.0)))?;
    window.set_size(saved.size)?;
    window.set_position(saved.position)?;
    if saved.maximized {
        window.maximize()?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_window_mode(state: State<'_, WindowMode>) -> Result<bool, String> {
    Ok(state.0.lock().map_err(|_| "窗口状态不可用")?.main.is_some())
}

#[tauri::command]
pub fn set_pet_mode(
    window: WebviewWindow,
    state: State<'_, WindowMode>,
    enabled: bool,
) -> Result<(), String> {
    let mut mode = state.0.lock().map_err(|_| "窗口状态不可用")?;
    if enabled == mode.main.is_some() {
        return Ok(());
    }
    if enabled {
        let saved = MainWindow {
            position: window.outer_position().map_err(|e| e.to_string())?,
            size: window.inner_size().map_err(|e| e.to_string())?,
            maximized: window.is_maximized().map_err(|e| e.to_string())?,
        };
        let monitor = window.current_monitor().map_err(|e| e.to_string())?;
        let result = (|| -> tauri::Result<()> {
            if saved.maximized {
                window.unmaximize()?;
            }
            window.set_min_size(None::<LogicalSize<f64>>)?;
            window.set_decorations(false)?;
            window.set_shadow(false)?;
            window.set_resizable(false)?;
            window.set_always_on_top(true)?;
            window.set_size(LogicalSize::new(PET_WIDTH, PET_HEIGHT))?;
            if let Some(monitor) = monitor {
                let area = monitor.work_area();
                let scale = monitor.scale_factor();
                let width = (PET_WIDTH * scale) as u32;
                let height = (PET_HEIGHT * scale) as u32;
                let preferred = mode.pet_position.unwrap_or_else(|| {
                    PhysicalPosition::new(
                        area.position.x + area.size.width as i32 - width as i32 - 24,
                        area.position.y + area.size.height as i32 - height as i32 - 24,
                    )
                });
                window.set_position(clamp_position(
                    preferred,
                    area.position,
                    area.size,
                    width,
                    height,
                ))?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = restore(&window, &saved);
            return Err(error.to_string());
        }
        mode.main = Some(saved);
    } else if let Some(saved) = &mode.main {
        let position = window.outer_position().map_err(|e| e.to_string())?;
        restore(&window, saved).map_err(|e| e.to_string())?;
        mode.pet_position = Some(position);
        mode.main = None;
    }
    Ok(())
}

#[tauri::command]
pub fn drag_pet(
    window: WebviewWindow,
    state: State<'_, WindowMode>,
    offset_x: f64,
    offset_y: f64,
) -> Result<(), String> {
    if !(0.0..=PET_WIDTH).contains(&offset_x) || !(0.0..=PET_HEIGHT).contains(&offset_y) {
        return Err("无效的拖动位置".into());
    }
    if state.0.lock().map_err(|_| "窗口状态不可用")?.main.is_none() {
        return Ok(());
    }
    let cursor = window.cursor_position().map_err(|e| e.to_string())?;
    if let Some(monitor) = window
        .monitor_from_point(cursor.x, cursor.y)
        .map_err(|e| e.to_string())?
    {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let position = PhysicalPosition::new(
            (cursor.x - offset_x * scale).round() as i32,
            (cursor.y - offset_y * scale).round() as i32,
        );
        window
            .set_position(clamp_position(
                position,
                area.position,
                area.size,
                (PET_WIDTH * scale) as u32,
                (PET_HEIGHT * scale) as u32,
            ))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn clamp_position(
    preferred: PhysicalPosition<i32>,
    origin: PhysicalPosition<i32>,
    area: PhysicalSize<u32>,
    width: u32,
    height: u32,
) -> PhysicalPosition<i32> {
    PhysicalPosition::new(
        preferred
            .x
            .clamp(origin.x, origin.x + area.width.saturating_sub(width) as i32),
        preferred.y.clamp(
            origin.y,
            origin.y + area.height.saturating_sub(height) as i32,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pet_stays_on_monitor_with_negative_coordinates_and_scaling() {
        assert_eq!(
            clamp_position(
                PhysicalPosition::new(2000, -100),
                PhysicalPosition::new(-1920, 0),
                PhysicalSize::new(1920, 1040),
                360,
                450
            ),
            PhysicalPosition::new(-360, 0)
        );
        assert_eq!(
            clamp_position(
                PhysicalPosition::new(-100, 2000),
                PhysicalPosition::new(0, 0),
                PhysicalSize::new(1920, 1040),
                240,
                300
            ),
            PhysicalPosition::new(0, 740)
        );
    }
}
