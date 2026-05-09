use smithay_client_toolkit::output::{OutputHandler, OutputInfo, OutputState};
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::wl_output;
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::{delegate_output, delegate_registry, registry_handlers};
use xcb::XidNew;

use crate::DisplayInfo;
use crate::error::{DIError, DIResult};

impl From<&OutputInfo> for DisplayInfo {
    fn from(info: &OutputInfo) -> Self {
        // Use the current mode's pixel dimensions for the true physical resolution.
        // The logical_size from xdg-output is already divided by the fractional
        // scale factor, so using it directly (as the original code did) and then
        // dividing again by wl_output's integer scale produces a doubly-scaled value.
        let (width, height) = info
            .modes
            .iter()
            .find(|m| m.current)
            .map(|m| (m.dimensions.0 as u32, m.dimensions.1 as u32))
            .unwrap_or_else(|| {
                let (w, h) = info.logical_size.unwrap_or(info.physical_size);
                (w as u32, h as u32)
            });

        // Derive the true (possibly fractional) scale factor from the ratio of
        // physical pixels to logical pixels. wl_output::scale is always an integer
        // and does not reflect compositor-level fractional scaling.
        // Round to 2 decimal places: the Wayland protocol truncates logical_size
        // to integers, which introduces a small error (e.g. 2560/1969 = 1.30015
        // instead of 1.30). Since scale factors are always set in whole percentages,
        // rounding to 2 decimal places recovers the exact value.
        let scale_factor = info
            .logical_size
            .and_then(|(lw, _)| {
                info.modes
                    .iter()
                    .find(|m| m.current)
                    .filter(|_| lw != 0)
                    .map(|m| (m.dimensions.0 as f32 / lw as f32 * 100.0).round() / 100.0)
            })
            .unwrap_or(info.scale_factor as f32);

        let rotation = match info.transform {
            wl_output::Transform::_90 | wl_output::Transform::Flipped90 => 90.,
            wl_output::Transform::_180 | wl_output::Transform::Flipped180 => 180.,
            wl_output::Transform::_270 | wl_output::Transform::Flipped270 => 270.,
            _ => 0.,
        };
        let frequency = info
            .modes
            .iter()
            .find(|m| m.current || m.preferred)
            .map(|m| m.refresh_rate as f32 / 1000.0)
            .unwrap_or(0.);

        // logical_position is already in logical pixels; no further scaling needed.
        let (x, y) = info.logical_position.unwrap_or(info.location);
        let (width_mm, height_mm) = info.physical_size;

        DisplayInfo {
            id: info.id,
            name: info.name.clone().unwrap_or_default(),
            friendly_name: info
                .name
                .clone()
                .unwrap_or(format!("Unknown Display {}", info.id)),
            raw_handle: xcb::randr::Output::new(info.id),
            x,
            y,
            width,
            height,
            width_mm,
            height_mm,
            rotation,
            scale_factor,
            frequency,
            is_primary: false, // resolved in get_all()
            is_builtin: false,
        }
    }
}

/// Application data.
struct ListOutputs {
    registry_state: RegistryState,
    output_state: OutputState,
}

impl OutputHandler for ListOutputs {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

delegate_output!(ListOutputs);
delegate_registry!(ListOutputs);

impl ProvidesRegistryState for ListOutputs {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers! {
        OutputState,
    }
}

pub fn get_all() -> DIResult<Vec<DisplayInfo>> {
    let conn = Connection::connect_to_env()?;

    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let registry_state = RegistryState::new(&globals);

    let output_delegate = OutputState::new(&globals, &qh);

    let mut list_outputs = ListOutputs {
        registry_state,
        output_state: output_delegate,
    };

    event_queue.roundtrip(&mut list_outputs)?;

    let mut infos = list_outputs
        .output_state
        .outputs()
        .map(|output| {
            list_outputs
                .output_state
                .info(&output)
                .map(|o| DisplayInfo::from(&o))
                .ok_or(DIError::new("Cannot get info from Output in Wayland"))
        })
        .collect::<DIResult<Vec<DisplayInfo>>>()?;

    // Wayland has no primary output concept. Mark the display closest to the
    // compositor origin as primary, which matches common desktop conventions.
    if let Some(primary) = infos.iter_mut().min_by_key(|d| d.x.abs() + d.y.abs()) {
        primary.is_primary = true;
    }

    Ok(infos)
}

pub fn get_from_point(x: i32, y: i32) -> DIResult<DisplayInfo> {
    let display_infos = get_all()?;

    display_infos
        .iter()
        .find(|&d| {
            x >= d.x
                && x - (d.width as i32) < d.x + d.width as i32
                && y >= d.y
                && y - (d.height as i32) < d.y + d.height as i32
        })
        .cloned()
        .ok_or_else(|| DIError::new("Get display info failed"))
}
