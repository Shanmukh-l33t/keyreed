//! UI Automation service (plan P4.1): read the text of the focused field, learn what kind of window it sits in, and be
//! told when the text changes.
//!
//! * **One thread, created at the first request.** Nothing exists before that and nothing runs between requests: the
//!   worker sleeps on its queue. COM is entered multithreaded on it.
//! * **A hung target never blocks another thread.** Callers hand a job to the worker and wait a bounded time for the
//!   answer ([`CALL_WAIT`]); a full queue answers `Busy` at once. The worker first sends the target window a `WM_NULL`
//!   and gives up after [`PROBE_MS`] when nobody answers; cross-process calls run with lowered connection and
//!   transaction timeouts ([`UIA_TIMEOUT_MS`], the default is two seconds); and a worker that still has not answered
//!   after [`CALL_WAIT`] is abandoned (it ends when its call returns) and the next request starts a fresh one, at most
//!   [`MAX_WEDGED`] abandoned workers at a time.
//! * **Never reads a password field.** The check comes before any text is touched.
//! * **Text never leaves this module except through the return value of [`Service::read_text`];** nothing is logged.
//!
//! The text pattern is read-only: this service cannot replace text.

use crossbeam_channel::{bounded, Receiver, Sender};
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::core::{implement, Interface, BSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationElement, IUIAutomationEventHandler, IUIAutomationEventHandler_Impl, IUIAutomationPropertyChangedEventHandler,
    IUIAutomationPropertyChangedEventHandler_Impl, IUIAutomationTextPattern, IUIAutomationValuePattern, TreeScope_Element, UIA_EVENT_ID, UIA_PROPERTY_ID, UIA_TextPatternId,
    UIA_Text_TextChangedEventId, UIA_ValuePatternId, UIA_ValueValuePropertyId,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, IsWindow, SendMessageTimeoutW, GA_ROOT, GUITHREADINFO, SMTO_ABORTIFHUNG, WM_NULL};

/// How long a caller waits for the worker before giving up.
pub const CALL_WAIT: Duration = Duration::from_millis(800);
/// Cross-process connection and transaction timeout inside the worker (UI Automation's default is 2000).
pub const UIA_TIMEOUT_MS: u32 = 250;
/// How long a target window gets to answer a `WM_NULL` before it counts as hung.
pub const PROBE_MS: u32 = 150;
/// Jobs that may wait for the worker; one more is refused with [`UiaError::Busy`].
const QUEUE: usize = 8;
/// Abandoned workers (still inside a call to a hung target) that may exist at once.
const MAX_WEDGED: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiaError {
    /// The worker did not answer in time, or the target did not answer the worker.
    Timeout,
    /// Too many jobs are waiting.
    Busy,
    /// A password field: never read.
    Password,
    /// The element has neither a text pattern nor a value pattern.
    Unsupported,
    /// There is no such element (the window is gone, nothing has focus).
    Gone,
    Failed(String),
}

impl fmt::Display for UiaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("the application did not answer in time"),
            Self::Busy => f.write_str("UI Automation is busy"),
            Self::Password => f.write_str("password field"),
            Self::Unsupported => f.write_str("the field does not expose its text"),
            Self::Gone => f.write_str("no such element"),
            Self::Failed(why) => write!(f, "UI Automation failed: {why}"),
        }
    }
}

impl std::error::Error for UiaError {}

/// Which element a request is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The element that has keyboard focus.
    Focused,
    /// The element behind a window handle (a top-level window or a control).
    Window(isize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementInfo {
    /// Class name of the top-level window the element sits in (`Chrome_WidgetWin_1`, `Notepad`, ...).
    pub window_class: String,
    pub process_id: u32,
    pub is_password: bool,
    /// The element's own window handle, 0 when it has none (Chromium).
    pub hwnd: isize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRead {
    pub text: String,
    /// The field held more than the cap.
    pub truncated: bool,
}

/// Identifies one text-changed subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WatchId(u64);

type Notify = Arc<dyn Fn() + Send + Sync>;

enum Job {
    Inspect(Target, Sender<Result<ElementInfo, UiaError>>),
    Read(Target, usize, Sender<Result<TextRead, UiaError>>),
    Watch(Target, Notify, Sender<Result<WatchId, UiaError>>),
    Unwatch(WatchId),
    /// Test only: a worker stuck inside a call.
    #[cfg(test)]
    Stall(Duration, Sender<Result<(), UiaError>>),
}

/// A UI Automation worker. Most code uses the process-wide one through the free functions below.
pub struct Service {
    /// The current worker's queue; `None` before the first request and after a worker was abandoned.
    queue: Mutex<Option<Sender<Job>>>,
    started: AtomicBool,
    /// Worker threads that exist (the current one and abandoned ones still inside a call).
    alive: std::sync::OnceLock<Arc<AtomicUsize>>,
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

/// Counts a worker thread while it lives.
struct Alive(Arc<AtomicUsize>);

impl Drop for Alive {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Service {
    pub const fn new() -> Self {
        Self { queue: Mutex::new(None), started: AtomicBool::new(false), alive: std::sync::OnceLock::new() }
    }

    /// Whether a worker thread has been created.
    pub fn is_running(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    fn submit(&self, job: Job) -> Result<(), UiaError> {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if queue.is_none() {
            let alive = self.alive.get_or_init(|| Arc::new(AtomicUsize::new(0)));
            if alive.load(Ordering::Acquire) > MAX_WEDGED {
                return Err(UiaError::Busy);
            }
            let (tx, rx) = bounded(QUEUE);
            alive.fetch_add(1, Ordering::AcqRel);
            let guard = Alive(alive.clone());
            self.started.store(true, Ordering::Release);
            // A failure to spawn drops the guard and the receiver: the job below is refused and callers see `Busy`.
            let _ = std::thread::Builder::new().name("keyreed-uia".into()).spawn(move || {
                let _alive = guard;
                worker(rx);
            });
            *queue = Some(tx);
        }
        queue.as_ref().expect("set above").try_send(job).map_err(|_| UiaError::Busy)
    }

    fn call<T>(&self, make: impl FnOnce(Sender<Result<T, UiaError>>) -> Job) -> Result<T, UiaError> {
        let (tx, rx) = bounded(1);
        self.submit(make(tx))?;
        match rx.recv_timeout(CALL_WAIT) {
            Ok(result) => result,
            Err(_) => {
                // The worker is stuck in a call to a target that does not answer: leave it to end by itself.
                self.queue.lock().unwrap_or_else(|e| e.into_inner()).take();
                Err(UiaError::Timeout)
            }
        }
    }

    /// What the element is: its window class, process, whether it is a password field.
    pub fn inspect(&self, target: Target) -> Result<ElementInfo, UiaError> {
        self.call(|reply| Job::Inspect(target, reply))
    }

    /// The text of the element, at most `max_chars` characters. Refuses password fields.
    pub fn read_text(&self, target: Target, max_chars: usize) -> Result<TextRead, UiaError> {
        self.call(|reply| Job::Read(target, max_chars, reply))
    }

    /// Calls `notify` (from a UI Automation thread: keep it short, send a message and return) when the element's text
    /// changes. Refuses password fields.
    pub fn watch(&self, target: Target, notify: impl Fn() + Send + Sync + 'static) -> Result<WatchId, UiaError> {
        let notify: Notify = Arc::new(notify);
        self.call(|reply| Job::Watch(target, notify, reply))
    }

    pub fn unwatch(&self, id: WatchId) {
        let _ = self.submit(Job::Unwatch(id));
    }
}

static GLOBAL: Service = Service::new();

/// The process-wide service (its thread is created by the first request to it).
pub fn service() -> &'static Service {
    &GLOBAL
}

// ---------------------------------------------------------------------------------------------------------------
// The worker thread
// ---------------------------------------------------------------------------------------------------------------

#[implement(IUIAutomationEventHandler, IUIAutomationPropertyChangedEventHandler)]
struct ChangeHandler {
    notify: Notify,
}

impl IUIAutomationEventHandler_Impl for ChangeHandler_Impl {
    fn HandleAutomationEvent(&self, _sender: windows::core::Ref<'_, IUIAutomationElement>, _event: UIA_EVENT_ID) -> windows::core::Result<()> {
        (self.notify)();
        Ok(())
    }
}

impl IUIAutomationPropertyChangedEventHandler_Impl for ChangeHandler_Impl {
    fn HandlePropertyChangedEvent(&self, _sender: windows::core::Ref<'_, IUIAutomationElement>, _property: UIA_PROPERTY_ID, _value: &VARIANT) -> windows::core::Result<()> {
        (self.notify)();
        Ok(())
    }
}

struct Watch {
    element: IUIAutomationElement,
    text_handler: IUIAutomationEventHandler,
    value_handler: IUIAutomationPropertyChangedEventHandler,
}

static NEXT_WATCH: AtomicU64 = AtomicU64::new(1);

fn worker(jobs: Receiver<Job>) {
    // SAFETY: COM entered once on this thread, for the life of the worker.
    let automation: Option<IUIAutomation> = unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
    };
    if let Some(a) = &automation {
        // SAFETY: plain COM calls; older systems without IUIAutomation2 keep the default timeouts.
        unsafe {
            if let Ok(a2) = a.cast::<IUIAutomation2>() {
                let _ = a2.SetConnectionTimeout(UIA_TIMEOUT_MS);
                let _ = a2.SetTransactionTimeout(UIA_TIMEOUT_MS);
            }
        }
    }
    let mut watches: HashMap<WatchId, Watch> = HashMap::new();
    for job in &jobs {
        let Some(automation) = &automation else {
            fail_job(job);
            continue;
        };
        // SAFETY: COM calls on interfaces this thread owns.
        unsafe {
            match job {
                Job::Inspect(target, reply) => {
                    let _ = reply.send(element(automation, target).and_then(|e| info(&e)));
                }
                Job::Read(target, max, reply) => {
                    let _ = reply.send(element(automation, target).and_then(|e| read(&e, max)));
                }
                Job::Watch(target, notify, reply) => {
                    let _ = reply.send(element(automation, target).and_then(|e| subscribe(automation, e, notify, &mut watches)));
                }
                #[cfg(test)]
                Job::Stall(how_long, reply) => {
                    std::thread::sleep(how_long);
                    let _ = reply.send(Ok(()));
                }
                Job::Unwatch(id) => {
                    if let Some(w) = watches.remove(&id) {
                        let _ = automation.RemoveAutomationEventHandler(UIA_Text_TextChangedEventId, &w.element, &w.text_handler);
                        let _ = automation.RemovePropertyChangedEventHandler(&w.element, &w.value_handler);
                    }
                }
            }
        }
    }
    // The queue is closed (the service was dropped or this worker was abandoned): leave no handlers behind.
    if let Some(a) = &automation {
        // SAFETY: plain COM call on this thread's automation object.
        unsafe {
            let _ = a.RemoveAllEventHandlers();
        }
    }
}

/// UI Automation could not be created: answer every job with a failure so no caller waits for nothing.
fn fail_job(job: Job) {
    let why = UiaError::Failed("UI Automation is not available".into());
    match job {
        Job::Inspect(_, r) => drop(r.send(Err(why))),
        Job::Read(_, _, r) => drop(r.send(Err(why))),
        Job::Watch(_, _, r) => drop(r.send(Err(why))),
        Job::Unwatch(_) => {}
        #[cfg(test)]
        Job::Stall(_, r) => drop(r.send(Err(why))),
    }
}

fn failed(e: windows::core::Error) -> UiaError {
    // 0x80040200 (event not found... timeouts) and the cross-process timeout codes read as a hung target.
    const RPC_TIMEOUT: [i32; 3] = [0x800706BEu32 as i32, 0x80131505u32 as i32, 0x8001010Au32 as i32];
    if RPC_TIMEOUT.contains(&e.code().0) {
        UiaError::Timeout
    } else if e.code().0 == 0x80040201u32 as i32 {
        UiaError::Gone // UIA_E_ELEMENTNOTAVAILABLE
    } else {
        UiaError::Failed(format!("{:#x}", e.code().0))
    }
}

/// Does the window's thread answer a message? A hung application does not, and every UI Automation call into it would
/// wait. The target of `Focused` is the focused control of the foreground thread, else the foreground window.
unsafe fn responsive(target: Target) -> bool {
    let hwnd = match target {
        Target::Window(h) => h as *mut core::ffi::c_void,
        Target::Focused => {
            let mut gui: GUITHREADINFO = std::mem::zeroed();
            gui.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
            if GetGUIThreadInfo(0, &mut gui) != 0 && !gui.hwndFocus.is_null() {
                gui.hwndFocus
            } else {
                GetForegroundWindow()
            }
        }
    };
    if hwnd.is_null() {
        return true; // nothing to probe: UI Automation will say there is no such element
    }
    let mut result = 0usize;
    SendMessageTimeoutW(hwnd, WM_NULL, 0, 0, SMTO_ABORTIFHUNG, PROBE_MS, &mut result) != 0
}

unsafe fn element(automation: &IUIAutomation, target: Target) -> Result<IUIAutomationElement, UiaError> {
    if let Target::Window(h) = target {
        if IsWindow(h as *mut _) == 0 {
            return Err(UiaError::Gone);
        }
    }
    if !responsive(target) {
        return Err(UiaError::Timeout);
    }
    match target {
        Target::Focused => automation.GetFocusedElement(),
        Target::Window(h) => automation.ElementFromHandle(HWND(h as *mut _)),
    }
    .map_err(|e| if matches!(target, Target::Focused) && !matches!(failed(e.clone()), UiaError::Timeout) { UiaError::Gone } else { failed(e) })
}

unsafe fn info(e: &IUIAutomationElement) -> Result<ElementInfo, UiaError> {
    let hwnd = e.CurrentNativeWindowHandle().map(|h| h.0 as isize).unwrap_or(0);
    let root = if hwnd != 0 { GetAncestor(hwnd as *mut _, GA_ROOT) } else { std::ptr::null_mut() };
    let root = if root.is_null() { GetForegroundWindow() } else { root };
    let mut class = [0u16; 256];
    let len = if root.is_null() { 0 } else { GetClassNameW(root, class.as_mut_ptr(), class.len() as i32).max(0) as usize };
    Ok(ElementInfo {
        window_class: String::from_utf16_lossy(&class[..len]),
        process_id: e.CurrentProcessId().map_err(failed)? as u32,
        is_password: e.CurrentIsPassword().map_err(failed)?.as_bool(),
        hwnd,
    })
}

unsafe fn read(e: &IUIAutomationElement, max_chars: usize) -> Result<TextRead, UiaError> {
    if e.CurrentIsPassword().map_err(failed)?.as_bool() {
        return Err(UiaError::Password);
    }
    // One more than the cap tells whether the field held more.
    let ask = max_chars.saturating_add(1);
    let text: BSTR = if let Ok(pattern) = e.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
        pattern.DocumentRange().and_then(|r| r.GetText(ask.min(i32::MAX as usize) as i32)).map_err(failed)?
    } else if let Ok(pattern) = e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
        pattern.CurrentValue().map_err(failed)?
    } else {
        return Err(UiaError::Unsupported);
    };
    let mut text = text.to_string();
    let truncated = text.chars().count() > max_chars;
    if truncated {
        text = text.chars().take(max_chars).collect();
    }
    Ok(TextRead { text, truncated })
}

unsafe fn subscribe(automation: &IUIAutomation, element: IUIAutomationElement, notify: Notify, watches: &mut HashMap<WatchId, Watch>) -> Result<WatchId, UiaError> {
    if element.CurrentIsPassword().map_err(failed)?.as_bool() {
        return Err(UiaError::Password);
    }
    let handler = ChangeHandler { notify };
    let text_handler: IUIAutomationEventHandler = handler.into();
    // The same object serves both interfaces; `Interface::cast` gives the second view of it.
    let value_handler: IUIAutomationPropertyChangedEventHandler = text_handler.cast().map_err(failed)?;
    // Text-changed events need a text pattern; value changes cover plain edit controls. Either may be refused.
    let text_ok = automation.AddAutomationEventHandler(UIA_Text_TextChangedEventId, &element, TreeScope_Element, None, &text_handler).is_ok();
    let value_ok = automation.AddPropertyChangedEventHandlerNativeArray(&element, TreeScope_Element, None, &value_handler, &[UIA_ValueValuePropertyId]).is_ok();
    if !text_ok && !value_ok {
        return Err(UiaError::Unsupported);
    }
    let id = WatchId(NEXT_WATCH.fetch_add(1, Ordering::Relaxed));
    watches.insert(id, Watch { element, text_handler, value_handler });
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;
    use windows_sys::Win32::Foundation::{HWND as SysHwnd, LPARAM, WPARAM};
    use windows_sys::Win32::UI::Accessibility::NotifyWinEvent;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW, PeekMessageW, PostMessageW, SendMessageW, ShowWindow, TranslateMessage, ES_MULTILINE, ES_PASSWORD, MSG,
        SW_SHOWNOACTIVATE, WM_QUIT, WM_SETTEXT, WS_BORDER, WS_POPUP, WS_VISIBLE,
    };

    /// Tests that start workers run one at a time, so the thread count means something.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    const EVENT_OBJECT_VALUECHANGE: u32 = 0x800E;
    const OBJID_CLIENT: i32 = -4;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A popup window holding an edit control: off screen, never activated.
    unsafe fn edit_window(text: &str, style: u32) -> SysHwnd {
        let (class, text) = (wide("EDIT"), wide(text));
        let hwnd = CreateWindowExW(0, class.as_ptr(), text.as_ptr(), WS_POPUP | WS_BORDER | WS_VISIBLE | style, -32000, -32000, 300, 100, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null());
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        hwnd
    }

    /// A test-owned edit control on its own thread, which pumps messages until the value is dropped.
    struct EditWindow {
        hwnd: isize,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl EditWindow {
        fn new(text: &str, style: u32) -> Self {
            let (tx, rx) = bounded(1);
            let text = text.to_string();
            let thread = std::thread::spawn(move || unsafe {
                let hwnd = edit_window(&text, style);
                tx.send(hwnd as isize).unwrap();
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                DestroyWindow(hwnd);
            });
            Self { hwnd: rx.recv().unwrap(), thread: Some(thread) }
        }

        fn set_text(&self, text: &str) {
            let w = wide(text);
            unsafe { SendMessageW(self.hwnd as SysHwnd, WM_SETTEXT, 0 as WPARAM, w.as_ptr() as LPARAM) };
            // A plain edit control raises no value event for WM_SETTEXT: tell the system the way an application would.
            unsafe { NotifyWinEvent(EVENT_OBJECT_VALUECHANGE, self.hwnd as SysHwnd, OBJID_CLIENT, 0) };
        }
    }

    impl Drop for EditWindow {
        fn drop(&mut self) {
            // Ends the thread's message loop.
            unsafe { PostMessageW(self.hwnd as SysHwnd, WM_QUIT, 0, 0) };
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// Threads of this process that carry the worker's name (thread descriptions exist from Windows 10 1607).
    fn uia_threads() -> usize {
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32};
        use windows_sys::Win32::System::Threading::{GetCurrentProcessId, GetThreadDescription, OpenThread, THREAD_QUERY_LIMITED_INFORMATION};
        let mut count = 0;
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            let mut ok = Thread32First(snap, &mut entry);
            while ok != 0 {
                if entry.th32OwnerProcessID == GetCurrentProcessId() {
                    let h = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, entry.th32ThreadID);
                    if !h.is_null() {
                        let mut name: *mut u16 = std::ptr::null_mut();
                        if GetThreadDescription(h, &mut name) >= 0 && !name.is_null() {
                            let len = (0..).take_while(|&i| *name.add(i) != 0).count();
                            if String::from_utf16_lossy(std::slice::from_raw_parts(name, len)) == "keyreed-uia" {
                                count += 1;
                            }
                        }
                        windows_sys::Win32::Foundation::CloseHandle(h);
                    }
                }
                ok = Thread32Next(snap, &mut entry);
            }
            windows_sys::Win32::Foundation::CloseHandle(snap);
        }
        count
    }

    #[test]
    fn no_thread_before_the_first_request_then_exactly_one() {
        let _one_at_a_time = serial();
        // Workers of earlier tests end when their service is dropped; wait until the count settles.
        let before = loop {
            let c = uia_threads();
            std::thread::sleep(Duration::from_millis(150));
            if c == uia_threads() {
                break c;
            }
        };
        let svc = Service::new();
        assert!(!svc.is_running());
        assert_eq!(uia_threads(), before, "creating a service must not start a thread");
        let w = EditWindow::new("hello", 0);
        svc.inspect(Target::Window(w.hwnd)).unwrap();
        assert!(svc.is_running());
        svc.inspect(Target::Window(w.hwnd)).unwrap();
        assert_eq!(uia_threads(), before + 1, "one worker, however many requests");
    }

    #[test]
    fn reads_inspects_and_caps_a_test_owned_edit_control() {
        let _one_at_a_time = serial();
        let svc = Service::new();
        let w = EditWindow::new("Hello w\u{f6}rld, this is a field", ES_MULTILINE as u32);
        let info = svc.inspect(Target::Window(w.hwnd)).unwrap();
        assert_eq!(info.window_class, "Edit");
        assert!(!info.is_password);
        assert_eq!(info.process_id, std::process::id());
        assert_eq!(svc.read_text(Target::Window(w.hwnd), 1000).unwrap(), TextRead { text: "Hello w\u{f6}rld, this is a field".into(), truncated: false });
        assert_eq!(svc.read_text(Target::Window(w.hwnd), 5).unwrap(), TextRead { text: "Hello".into(), truncated: true });
        assert_eq!(svc.read_text(Target::Window(0x10), 5), Err(UiaError::Gone), "a handle that is no window");
    }

    #[test]
    fn password_fields_are_never_read() {
        let _one_at_a_time = serial();
        let svc = Service::new();
        let w = EditWindow::new("hunter2", ES_PASSWORD as u32);
        assert!(svc.inspect(Target::Window(w.hwnd)).unwrap().is_password);
        assert_eq!(svc.read_text(Target::Window(w.hwnd), 100), Err(UiaError::Password));
        assert_eq!(svc.watch(Target::Window(w.hwnd), || {}).map(|_| ()), Err(UiaError::Password));
    }

    #[test]
    fn watch_is_told_when_the_text_changes_and_stops_after_unwatch() {
        let _one_at_a_time = serial();
        let svc = Service::new();
        let w = EditWindow::new("before", 0);
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let id = svc
            .watch(Target::Window(w.hwnd), move || {
                h.fetch_add(1, Ordering::SeqCst);
            })
            .unwrap();
        w.set_text("after");
        let start = Instant::now();
        while hits.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(hits.load(Ordering::SeqCst) >= 1, "no change notification within 3 s");
        assert_eq!(svc.read_text(Target::Window(w.hwnd), 100).unwrap().text, "after");
        svc.unwatch(id);
        std::thread::sleep(Duration::from_millis(300));
        let settled = hits.load(Ordering::SeqCst);
        w.set_text("again");
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(hits.load(Ordering::SeqCst), settled, "notifications after unwatch");
    }

    /// Not a test of its own: the other process of `a_hung_target_...`. It owns a window and never answers a message.
    #[test]
    fn hung_window_process() {
        if std::env::var_os("KEYREED_UIA_TEST_HUNG_WINDOW").is_none() {
            return;
        }
        unsafe {
            let hwnd = edit_window("stuck", 0);
            println!("HWND {}", hwnd as isize);
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut msg: MSG = std::mem::zeroed();
            PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, 0); // creates the queue, then never pumps it
            std::thread::sleep(Duration::from_secs(8));
        }
    }

    #[test]
    fn a_worker_stuck_in_a_call_is_abandoned_and_replaced() {
        let _one_at_a_time = serial();
        let svc = Service::new();
        let w = EditWindow::new("fine", 0);
        svc.inspect(Target::Window(w.hwnd)).unwrap();
        let t = Instant::now();
        let stuck: Result<(), UiaError> = svc.call(|reply| Job::Stall(Duration::from_secs(2), reply));
        assert_eq!(stuck, Err(UiaError::Timeout));
        assert!(t.elapsed() < CALL_WAIT + Duration::from_millis(400), "the caller waited {:?}", t.elapsed());
        // The next request does not queue behind the stuck call: a fresh worker answers it.
        let t = Instant::now();
        assert_eq!(svc.read_text(Target::Window(w.hwnd), 100).unwrap().text, "fine");
        assert!(t.elapsed() < Duration::from_millis(700), "the replacement worker took {:?}", t.elapsed());
        // Repeated hangs never pile up more than MAX_WEDGED stuck threads: past that, requests are refused at once.
        for _ in 0..MAX_WEDGED + 2 {
            let (reply, _unread) = bounded(1);
            if svc.submit(Job::Stall(Duration::from_secs(4), reply)).is_ok() {
                svc.queue.lock().unwrap().take(); // abandon it without waiting
            }
        }
        let t = Instant::now();
        assert_eq!(svc.inspect(Target::Window(w.hwnd)), Err(UiaError::Busy));
        assert!(t.elapsed() < Duration::from_millis(50));
        // Let the stuck threads end before the next test counts threads.
        drop(svc);
        let start = Instant::now();
        while uia_threads() > 0 && start.elapsed() < Duration::from_secs(8) {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(uia_threads(), 0, "stuck workers did not end");
    }

    #[test]
    fn a_hung_target_costs_one_bounded_wait_and_blocks_no_other_thread() {
        let _one_at_a_time = serial();
        // The hung window lives in a child process (the test binary itself), as a real hung application would.
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["uia::tests::hung_window_process", "--exact", "--nocapture", "--test-threads=1"])
            .env("KEYREED_UIA_TEST_HUNG_WINDOW", "1")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let hung: isize = BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok).find_map(|l| l.split_once("HWND ").and_then(|(_, n)| n.trim().parse().ok())).expect("the child reports its window");
        let svc = Arc::new(Service::new());
        let good = EditWindow::new("fine", 0);
        svc.inspect(Target::Window(good.hwnd)).unwrap(); // warm the worker: its start-up is not part of the measurement
        let started = Instant::now();
        let s2 = svc.clone();
        let reader = std::thread::spawn(move || s2.read_text(Target::Window(hung), 100));
        std::thread::sleep(Duration::from_millis(50));
        // While the worker waits on the hung window, other threads still come back within the bound.
        let t = Instant::now();
        let other = svc.inspect(Target::Window(good.hwnd));
        assert!(t.elapsed() <= CALL_WAIT + Duration::from_millis(300), "a second caller waited {:?}", t.elapsed());
        assert!(matches!(other, Ok(_) | Err(UiaError::Timeout) | Err(UiaError::Busy)), "{other:?}");
        let result = reader.join().unwrap();
        assert!(matches!(result, Err(UiaError::Timeout) | Err(UiaError::Failed(_)) | Err(UiaError::Gone)), "{result:?}");
        assert!(started.elapsed() < Duration::from_millis(2500), "the hung read took {:?}", started.elapsed());
        // The worker is free again once the target's timeout has passed.
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(svc.read_text(Target::Window(good.hwnd), 100).unwrap().text, "fine");
        let _ = child.kill();
        let _ = child.wait();
    }
}

