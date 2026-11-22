//! Transparency, animations and how windows look and behave while moved.

use std::mem::size_of;

use serde_json::Value;
use windows::Win32::UI::WindowsAndMessaging::{
    ANIMATIONINFO, SPI_GETCLIENTAREAANIMATION, SPI_GETDRAGFULLWINDOWS, SPI_GETDROPSHADOW,
    SPI_GETWINARRANGING, SPI_SETANIMATION, SPI_SETCLIENTAREAANIMATION, SPI_SETDRAGFULLWINDOWS,
    SPI_SETDROPSHADOW, SPI_SETWINARRANGING,
};

use super::{Values, as_param, flag, spi_get, spi_set};
use crate::win;

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

pub fn read(values: &mut Values) {
    let transparency = win::read_user_value(PERSONALIZE, "EnableTransparency")
        .and_then(|value| value.as_dword())
        .is_none_or(|value| value != 0);
    let switch = |action| spi_get::<i32>(action).map(|value| value != 0);
    values.insert("transparency".into(), transparency.into());
    values.insert("animations".into(), switch(SPI_GETCLIENTAREAANIMATION).into());
    values.insert("windowShadows".into(), switch(SPI_GETDROPSHADOW).into());
    values.insert("dragFullWindows".into(), switch(SPI_GETDRAGFULLWINDOWS).into());
    values.insert("snapWindows".into(), switch(SPI_GETWINARRANGING).into());
}

pub fn set(name: &str, value: &Value) -> Option<Result<(), String>> {
    let change: fn(bool) -> Result<(), String> = match name {
        "transparency" => set_transparency,
        "animations" => set_animations,
        "windowShadows" => |on| spi_set(SPI_SETDROPSHADOW, 0, as_param(on as u32)),
        "dragFullWindows" => |on| spi_set(SPI_SETDRAGFULLWINDOWS, on as u32, std::ptr::null_mut()),
        // The documentation names the pointer, but some builds read the other
        // argument, so both carry the value.
        "snapWindows" => |on| spi_set(SPI_SETWINARRANGING, on as u32, as_param(on as u32)),
        _ => return None,
    };
    Some(flag(value).and_then(change))
}

fn set_transparency(on: bool) -> Result<(), String> {
    win::set_user_dword(PERSONALIZE, "EnableTransparency", on as u32)?;
    win::broadcast_color_change();
    Ok(())
}

/// The Settings switch covers both the animations inside windows and the one
/// played when a window is minimized or restored.
fn set_animations(on: bool) -> Result<(), String> {
    spi_set(SPI_SETCLIENTAREAANIMATION, 0, as_param(on as u32))?;
    let mut info = ANIMATIONINFO {
        cbSize: size_of::<ANIMATIONINFO>() as u32,
        iMinAnimate: on as i32,
    };
    spi_set(SPI_SETANIMATION, info.cbSize, &mut info as *mut ANIMATIONINFO as *mut _)
}
