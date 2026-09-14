use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    rc::Rc,
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    battery_icon,
    debounce::{Debouncer, Kind},
    manager::DeviceManager,
    notify::Notify,
    options::Options,
    reminder::{Reminder, LOW_LEVEL},
    system,
    wake::{Activity, WakeRetry},
};
use log::{error, info, trace, warn};
use parking_lot::Mutex;
use tao::event_loop::{EventLoopBuilder, EventLoopProxy};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    TrayIcon, TrayIconBuilder,
};

/// Battery poll interval while every device is above the low level.
const BATTERY_UPDATE_INTERVAL: Duration = Duration::from_secs(300);
/// Faster polling while a device is low, so plugging in the cable stops
/// reminders quickly.
const BATTERY_UPDATE_INTERVAL_LOW: Duration = Duration::from_secs(60);
const DEVICE_FETCH_INTERVAL: Duration = Duration::from_secs(5);
const CONNECTION_DEBOUNCE: Duration = Duration::from_secs(10);

/// Fake device used by `--simulate-battery` when no mouse is plugged in.
const SIMULATED_PID: u32 = 0;

/// Windows truncates tray tooltips to 127 characters.
const TOOLTIP_MAX_CHARS: usize = 127;

#[derive(Debug)]
pub struct MemoryDevice {
    pub name: String,
    /// Last successfully read level; `None` until the first read.
    pub battery_level: Option<i32>,
    /// The last read failed (mouse asleep or out of range).
    pub stale: bool,
    pub is_charging: bool,
    full_notified: bool,
    reminder: Reminder,
    /// When to read again while the mouse doesn't answer.
    retry: WakeRetry,
}

impl MemoryDevice {
    fn new(name: String) -> Self {
        Self {
            name,
            battery_level: None,
            stale: false,
            is_charging: false,
            full_notified: false,
            reminder: Reminder::default(),
            retry: WakeRetry::default(),
        }
    }
}

enum WorkerCommand {
    Refresh,
}

pub struct TrayInner {
    tray_icon: Rc<Mutex<Option<TrayIcon>>>,
}

struct MenuItems {
    refresh: MenuItem,
    quit: MenuItem,
}

impl TrayInner {
    fn new() -> Self {
        Self {
            tray_icon: Rc::new(Mutex::new(None)),
        }
    }

    fn create_menu() -> (Menu, MenuItems) {
        let tray_menu = Menu::new();
        let items = MenuItems {
            refresh: MenuItem::new("Refresh now", true, None),
            quit: MenuItem::new("Exit", true, None),
        };

        if let Err(e) = tray_menu.append_items(&[
            &items.refresh,
            &PredefinedMenuItem::separator(),
            &items.quit,
        ]) {
            warn!("Failed to append menu items: {}", e);
        }
        (tray_menu, items)
    }

    fn build_tray(
        tray_icon: &Rc<Mutex<Option<TrayIcon>>>,
        tray_menu: &Menu,
        icon: tray_icon::Icon,
    ) {
        let tray_builder = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu.clone()))
            .with_tooltip("Searching for Razer devices…")
            .with_icon(icon)
            .build();

        match tray_builder {
            Ok(tray) => *tray_icon.lock() = Some(tray),
            Err(err) => error!("Failed to create tray icon: {}", err),
        }
    }
}

pub struct TrayApp {
    devices: Arc<Mutex<HashMap<u32, MemoryDevice>>>,
    tray_inner: TrayInner,
    notify: Arc<Notify>,
    options: Options,
}

#[derive(Debug)]
enum TrayEvent {
    DeviceUpdate,
    MenuEvent(MenuEvent),
}

impl TrayApp {
    pub fn new(options: Options) -> Self {
        Self {
            devices: Arc::new(Mutex::new(HashMap::new())),
            tray_inner: TrayInner::new(),
            notify: Arc::new(Notify::new()),
            options,
        }
    }

    pub fn run(&self) {
        let event_loop = EventLoopBuilder::with_user_event().build();
        let icon = match Self::get_battery_icon(None, false) {
            Ok(icon) => icon,
            Err(e) => {
                error!("{}", e);
                return;
            }
        };
        let (tray_menu, menu_items) = TrayInner::create_menu();

        let proxy = event_loop.create_proxy();
        let (cmd_tx, cmd_rx) = mpsc::channel();

        self.spawn_worker_thread(proxy.clone(), cmd_rx);

        self.run_event_loop(event_loop, icon, tray_menu, menu_items, proxy, cmd_tx);
    }

    fn spawn_worker_thread(
        &self,
        proxy: EventLoopProxy<TrayEvent>,
        cmd_rx: Receiver<WorkerCommand>,
    ) {
        let devices = Arc::clone(&self.devices);
        let notify = Arc::clone(&self.notify);
        let options = self.options;

        thread::spawn(move || {
            let mut manager = DeviceManager::new();
            let mut last_devices = HashSet::new();
            let mut debouncer = Debouncer::new(CONNECTION_DEBOUNCE);
            let mut next_battery_poll = Instant::now();
            let mut force_refresh = false;
            let mut last_light_theme = system::taskbar_is_light();
            let mut activity = Activity::default();
            let reminder_unit = if options.fast_reminders {
                Duration::from_secs(1)
            } else {
                Duration::from_secs(60)
            };

            loop {
                let (removed_devices, connected_devices) = manager.fetch_devices();
                let now = Instant::now();
                let mut ui_changed = false;

                let mut guard = devices.lock();

                for id in &removed_devices {
                    if let Some(device) = guard.remove(id) {
                        info!("Device removed: {}", device.name);
                        debouncer.push(Kind::Disconnected, &device.name, now);
                    }
                }

                for &id in &connected_devices {
                    if let Entry::Vacant(e) = guard.entry(id) {
                        if let Some(name) = manager.get_device_name(id) {
                            info!("New device: {}", name);
                            debouncer.push(Kind::Connected, &name, now);
                            e.insert(MemoryDevice::new(name));
                        } else {
                            error!("Failed to get device name for id: {}", id);
                        }
                    }
                }

                if options.simulate_battery.is_some() {
                    let has_real = guard.keys().any(|&id| id != SIMULATED_PID);
                    if !has_real && !guard.contains_key(&SIMULATED_PID) {
                        guard.insert(
                            SIMULATED_PID,
                            MemoryDevice::new("Simulated Razer Mouse".to_owned()),
                        );
                        ui_changed = true;
                    } else if has_real {
                        ui_changed |= guard.remove(&SIMULATED_PID).is_some();
                    }
                }

                // A mouse that stopped answering is retried as soon as the user
                // touches the PC again (moving the mouse wakes it), with backoff.
                let input = activity.sample(now, system::idle_time());
                if input.returned {
                    trace!("User input after idle, retrying sleeping devices");
                    guard.values_mut().for_each(|d| d.retry.reset_backoff());
                }

                let poll_due = force_refresh || now >= next_battery_poll;
                for (&id, device) in guard.iter_mut() {
                    let needs_first_read = device.battery_level.is_none() && !device.stale;
                    let wake_retry = device.stale && device.retry.due(now, input.last_input);
                    if poll_due || connected_devices.contains(&id) || needs_first_read || wake_retry
                    {
                        read_battery(&mut manager, id, device, options);
                        device.retry.record(now, !device.stale);
                        check_full(device, &notify);
                        ui_changed = true;
                    }
                }

                if poll_due {
                    force_refresh = false;
                    let any_low = guard
                        .values()
                        .any(|d| d.battery_level.is_some_and(|l| l <= LOW_LEVEL));
                    next_battery_poll = now
                        + if any_low {
                            BATTERY_UPDATE_INTERVAL_LOW
                        } else {
                            BATTERY_UPDATE_INTERVAL
                        };
                }

                // Reminders tick every loop, independent of battery polling.
                // Cheap check, but skip it entirely while nothing is low.
                let any_candidate = guard.values().any(|d| {
                    !d.stale && !d.is_charging && d.battery_level.is_some_and(|l| l <= LOW_LEVEL)
                });
                let game_running =
                    any_candidate && (options.simulate_game || system::fullscreen_app_running());
                for device in guard.values_mut() {
                    // While the mouse doesn't answer, freeze the schedule; a due
                    // reminder fires once it responds again.
                    if device.stale {
                        continue;
                    }
                    let Some(level) = device.battery_level else {
                        continue;
                    };
                    if let Some(alert) = device.reminder.update(
                        now,
                        level,
                        device.is_charging,
                        game_running,
                        reminder_unit,
                    ) {
                        info!(
                            "{}: battery {} ({}%), alert #{}",
                            device.name,
                            if alert.critical { "critical" } else { "low" },
                            level,
                            alert.count
                        );
                        notify.battery_low(&device.name, level, alert);
                    }
                }
                if game_running {
                    trace!("Fullscreen app running, holding reminders");
                }

                for (kind, name) in debouncer.drain_due(now) {
                    match kind {
                        Kind::Connected => notify.device_connected(&name),
                        Kind::Disconnected => notify.device_disconnected(&name),
                    }
                }

                let current_devices: HashSet<u32> = guard.keys().copied().collect();
                let light_theme = system::taskbar_is_light();
                if ui_changed || current_devices != last_devices || light_theme != last_light_theme
                {
                    let _ = proxy.send_event(TrayEvent::DeviceUpdate);
                }
                last_devices = current_devices;
                last_light_theme = light_theme;

                drop(guard);

                match cmd_rx.recv_timeout(DEVICE_FETCH_INTERVAL) {
                    Ok(WorkerCommand::Refresh) => {
                        info!("Manual refresh requested");
                        force_refresh = true;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
    }

    fn run_event_loop(
        &self,
        event_loop: tao::event_loop::EventLoop<TrayEvent>,
        icon: tray_icon::Icon,
        tray_menu: Menu,
        menu_items: MenuItems,
        proxy: EventLoopProxy<TrayEvent>,
        cmd_tx: Sender<WorkerCommand>,
    ) {
        let devices = Arc::clone(&self.devices);
        let tray_icon = Rc::clone(&self.tray_inner.tray_icon);

        let menu_channel = MenuEvent::receiver();

        event_loop.run(move |event, _, control_flow| {
            *control_flow = tao::event_loop::ControlFlow::Wait;

            match event {
                tao::event::Event::NewEvents(tao::event::StartCause::Init) => {
                    TrayInner::build_tray(&tray_icon, &tray_menu, icon.clone());
                }
                tao::event::Event::UserEvent(TrayEvent::DeviceUpdate) => {
                    Self::update_tray_ui(&devices, &tray_icon);
                }
                tao::event::Event::UserEvent(TrayEvent::MenuEvent(event)) => {
                    if event.id == menu_items.refresh.id() {
                        let _ = cmd_tx.send(WorkerCommand::Refresh);
                    }

                    if event.id == menu_items.quit.id() {
                        *control_flow = tao::event_loop::ControlFlow::Exit;
                    }
                }
                _ => (),
            }

            if let Ok(event) = menu_channel.try_recv() {
                let _ = proxy.send_event(TrayEvent::MenuEvent(event));
            }
        });
    }

    fn update_tray_ui(
        devices: &Arc<Mutex<HashMap<u32, MemoryDevice>>>,
        tray_icon: &Rc<Mutex<Option<TrayIcon>>>,
    ) {
        let guard = devices.lock();
        let mut tray = tray_icon.lock();
        let Some(tray) = tray.as_mut() else {
            return;
        };

        if guard.is_empty() {
            if let Ok(icon) = Self::get_battery_icon(None, false) {
                let _ = tray.set_icon(Some(icon));
            }
            let _ = tray.set_tooltip(Some("No Razer devices detected"));
            return;
        }

        let mut list: Vec<&MemoryDevice> = guard.values().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));

        // Icon: prefer a charging device (the same mouse on cable), else the
        // lowest battery among devices that answer.
        let shown = list
            .iter()
            .filter(|d| !d.stale && d.battery_level.is_some())
            .min_by_key(|d| (!d.is_charging, d.battery_level));
        let icon = match shown {
            Some(d) => Self::get_battery_icon(d.battery_level, d.is_charging),
            None => Self::get_battery_icon(None, false),
        };
        if let Ok(icon) = icon {
            let _ = tray.set_icon(Some(icon));
        }

        let tooltip = list
            .iter()
            .map(|d| tooltip_line(d))
            .collect::<Vec<_>>()
            .join("\n");
        let _ = tray.set_tooltip(Some(truncate_chars(&tooltip, TOOLTIP_MAX_CHARS)));
    }

    /// Battery glyph at the tray's pixel size, colored for the current
    /// taskbar theme. `None` draws the gray "unknown" state.
    fn get_battery_icon(level: Option<i32>, is_charging: bool) -> Result<tray_icon::Icon, String> {
        let size = system::tray_icon_size();
        let style = battery_icon::tray_style(level, is_charging, system::taskbar_is_light());
        let rgba = battery_icon::render(size, level.unwrap_or(0), style);

        tray_icon::Icon::from_rgba(rgba, size, size)
            .map_err(|e| format!("Failed to create icon: {}", e))
    }
}

fn read_battery(manager: &mut DeviceManager, id: u32, device: &mut MemoryDevice, options: Options) {
    let simulated = options.simulate_battery;
    let level = simulated.or_else(|| manager.get_device_battery_level(id));
    let charging = if simulated.is_some() {
        Some(false)
    } else if level.is_some() {
        manager.is_device_charging(id)
    } else {
        None
    };

    match level {
        Some(level) => {
            info!("{}  battery level: {}%", device.name, level);
            if device.battery_level.is_none() {
                // Don't announce "fully charged" for a mouse that already was.
                device.full_notified = level >= 100;
            }
            device.battery_level = Some(level);
            device.stale = false;
        }
        None => {
            if !device.stale {
                warn!("{}: no battery reading (asleep?)", device.name);
            }
            device.stale = true;
        }
    }
    if let Some(charging) = charging {
        info!("{}  charging status: {}", device.name, charging);
        device.is_charging = charging;
    }
}

fn check_full(device: &mut MemoryDevice, notify: &Notify) {
    let Some(level) = device.battery_level else {
        return;
    };
    if level < 100 {
        device.full_notified = false;
    } else if device.is_charging && !device.full_notified {
        info!("{}: Battery fully charged", device.name);
        notify.battery_full(&device.name);
        device.full_notified = true;
    }
}

fn tooltip_line(device: &MemoryDevice) -> String {
    let name = device.name.strip_prefix("Razer ").unwrap_or(&device.name);
    match (device.battery_level, device.stale) {
        (_, true) => format!("{name}: not responding"),
        (None, false) => format!("{name}: reading…"),
        (Some(level), false) if device.is_charging => format!("{name}: {level}% ⚡"),
        (Some(level), false) => format!("{name}: {level}%"),
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((idx, _)) => s[..idx].to_owned(),
        None => s.to_owned(),
    }
}
