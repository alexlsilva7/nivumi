//! UI Automation providers for the custom-rendered notch.
//!
//! Raw vtables are intentional: UIA requires successful null interface outputs
//! for missing patterns/navigation; the generated non-optional trait signatures
//! cannot express those results safely.
#![allow(non_snake_case)]
use std::{
    ffi::c_void,
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use windows::{
    core::{IUnknown, IUnknown_Vtbl, Interface, GUID, HRESULT},
    Win32::{
        Foundation::{E_NOINTERFACE, E_POINTER, HWND, LPARAM, POINT, RECT, S_OK, WPARAM},
        Graphics::Gdi::ClientToScreen,
        System::{
            Com::SAFEARRAY,
            Ole::{SafeArrayCreateVector, SafeArrayDestroy, SafeArrayPutElement},
            Variant::{VARIANT, VT_I4},
        },
        UI::{
            Accessibility::*,
            WindowsAndMessaging::{GetWindowRect, IsWindow, PostMessageW, WM_APP},
        },
    },
};

pub const ACCESSIBLE_INVOKE: u32 = WM_APP + 50;
pub const ACCESSIBLE_FOCUS: u32 = WM_APP + 51;

#[derive(Debug, Clone)]
pub struct AccessibleItem {
    pub label: String,
    /// Client coordinates in physical pixels, not DIP.
    pub rect: RECT,
    pub action_id: usize,
    pub enabled: bool,
}
struct Shared {
    hwnd: usize,
    items: Arc<Mutex<Vec<AccessibleItem>>>,
    focused: AtomicUsize,
    active: AtomicBool,
}
impl Shared {
    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut c_void)
    }
}

#[repr(C)]
struct Slot<V> {
    vtable: *const V,
    owner: *mut Element,
}
struct Element {
    simple: Slot<IRawElementProviderSimple_Vtbl>,
    fragment: Slot<IRawElementProviderFragment_Vtbl>,
    root: Slot<IRawElementProviderFragmentRoot_Vtbl>,
    invoke: Slot<IInvokeProvider_Vtbl>,
    refs: AtomicU32,
    shared: Arc<Shared>,
    action: Option<usize>,
    root_ref: Option<IRawElementProviderSimple>,
}
unsafe fn element<'a>(this: *mut c_void) -> &'a Element {
    &*(*(this as *mut Slot<IRawElementProviderSimple_Vtbl>)).owner
}
fn lock_items(shared: &Shared) -> std::sync::MutexGuard<'_, Vec<AccessibleItem>> {
    shared
        .items
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn unavailable() -> HRESULT {
    HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32)
}
fn item(e: &Element) -> Option<AccessibleItem> {
    if !e.shared.active.load(Ordering::Acquire) {
        return None;
    }
    let action = e.action?;
    lock_items(&e.shared)
        .iter()
        .find(|i| i.action_id == action)
        .cloned()
}
unsafe fn simple_ref(e: &Element) -> IRawElementProviderSimple {
    if let Some(root) = &e.root_ref {
        root.clone()
    } else {
        let raw = ptr::addr_of!(e.simple) as *mut c_void;
        add_ref(raw);
        IRawElementProviderSimple::from_raw(raw)
    }
}
unsafe fn make(
    shared: Arc<Shared>,
    action: Option<usize>,
    root_ref: Option<IRawElementProviderSimple>,
) -> IRawElementProviderSimple {
    let boxed = Box::new(Element {
        simple: Slot {
            vtable: &SIMPLE,
            owner: ptr::null_mut(),
        },
        fragment: Slot {
            vtable: &FRAGMENT,
            owner: ptr::null_mut(),
        },
        root: Slot {
            vtable: &ROOT,
            owner: ptr::null_mut(),
        },
        invoke: Slot {
            vtable: &INVOKE,
            owner: ptr::null_mut(),
        },
        refs: AtomicU32::new(1),
        shared,
        action,
        root_ref,
    });
    let raw = Box::into_raw(boxed);
    (*raw).simple.owner = raw;
    (*raw).fragment.owner = raw;
    (*raw).root.owner = raw;
    (*raw).invoke.owner = raw;
    IRawElementProviderSimple::from_raw(ptr::addr_of_mut!((*raw).simple).cast())
}

/// Cache this provider for the lifetime of HWND. Keep item action IDs stable.
/// The caller owns the HWND and publishes only screen-visible controls.
///
/// # Safety
/// `hwnd` must identify the caller's live window. Install the documented message
/// handlers and retain the provider until that window is destroyed.
pub unsafe fn provider(
    hwnd: HWND,
    items: Arc<Mutex<Vec<AccessibleItem>>>,
) -> IRawElementProviderSimple {
    make(
        Arc::new(Shared {
            hwnd: hwnd.0 as usize,
            items,
            focused: AtomicUsize::new(usize::MAX),
            active: AtomicBool::new(true),
        }),
        None,
        None,
    )
}

/// Synchronize keyboard focus after Tab/arrow navigation. Only pass a provider
/// created by this module. This also sends the corresponding UIA focus event.
///
/// # Safety
/// `provider` must have been created by this module's `provider` function.
pub unsafe fn keyboard_focus(provider: &IRawElementProviderSimple, action: Option<usize>) {
    let e = element(provider.as_raw());
    if !e.shared.active.load(Ordering::Acquire) {
        return;
    }
    let next = action.unwrap_or(usize::MAX);
    if e.shared.focused.swap(next, Ordering::Relaxed) == next {
        return;
    }
    if let Some(id) = action {
        let child = make(e.shared.clone(), Some(id), Some(simple_ref(e)));
        let _ = UiaRaiseAutomationEvent(&child, UIA_AutomationFocusChangedEventId);
    }
}

/// Call after the list of visible actions changes, not on every animation frame.
///
/// # Safety
/// Call from the owning UI thread without holding the items mutex; UIA can
/// synchronously query the provider while the event is being delivered.
pub unsafe fn notify_changed(provider: &IRawElementProviderSimple) {
    if !element(provider.as_raw())
        .shared
        .active
        .load(Ordering::Acquire)
    {
        return;
    }
    let _ = UiaRaiseStructureChangedEvent(
        provider,
        StructureChangeType_ChildrenInvalidated,
        ptr::null_mut(),
        0,
    );
}

/// Invalidate all retained root/child references before destroying the HWND.
///
/// # Safety
/// `provider` must originate from this module. Call on the window thread during
/// WM_DESTROY before the HWND can be reused for another window.
pub unsafe fn disconnect(provider: &IRawElementProviderSimple) {
    element(provider.as_raw())
        .shared
        .active
        .store(false, Ordering::Release);
    let _ = UiaDisconnectProvider(provider);
}

unsafe extern "system" fn query(
    this: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    *out = ptr::null_mut();
    let e = element(this);
    let raw = if *iid == IUnknown::IID || *iid == IRawElementProviderSimple::IID {
        ptr::addr_of!(e.simple).cast_mut().cast()
    } else if *iid == IRawElementProviderFragment::IID {
        ptr::addr_of!(e.fragment).cast_mut().cast()
    } else if *iid == IRawElementProviderFragmentRoot::IID && e.action.is_none() {
        ptr::addr_of!(e.root).cast_mut().cast()
    } else if *iid == IInvokeProvider::IID && e.action.is_some() {
        ptr::addr_of!(e.invoke).cast_mut().cast()
    } else {
        return E_NOINTERFACE;
    };
    add_ref(this);
    *out = raw;
    S_OK
}
unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    element(this).refs.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    let owner = (*(this as *mut Slot<IRawElementProviderSimple_Vtbl>)).owner;
    let count = (*owner).refs.fetch_sub(1, Ordering::Release) - 1;
    if count == 0 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(Box::from_raw(owner));
    }
    count
}
const fn unknown() -> IUnknown_Vtbl {
    IUnknown_Vtbl {
        QueryInterface: query,
        AddRef: add_ref,
        Release: release,
    }
}
unsafe extern "system" fn options(_: *mut c_void, out: *mut ProviderOptions) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = ProviderOptions_ServerSideProvider;
    S_OK
}
unsafe extern "system" fn pattern(
    this: *mut c_void,
    pattern: UIA_PATTERN_ID,
    out: *mut *mut c_void,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = ptr::null_mut();
    if !element(this).shared.active.load(Ordering::Acquire) {
        return unavailable();
    }
    if pattern == UIA_InvokePatternId && element(this).action.is_some() {
        return query(this, &IInvokeProvider::IID, out);
    }
    S_OK
}
unsafe extern "system" fn property(
    this: *mut c_void,
    id: UIA_PROPERTY_ID,
    out: *mut VARIANT,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    out.write(VARIANT::default());
    let e = element(this);
    if !e.shared.active.load(Ordering::Acquire) {
        return unavailable();
    }
    let current = item(e);
    if e.action.is_some() && current.is_none() {
        return unavailable();
    }
    let value = if id == UIA_NamePropertyId {
        VARIANT::from(
            current
                .as_ref()
                .map(|i| i.label.as_str())
                .unwrap_or("Nivumi"),
        )
    } else if id == UIA_HelpTextPropertyId {
        VARIANT::from(current.as_ref().map(|i| i.label.as_str()).unwrap_or(""))
    } else if id == UIA_AutomationIdPropertyId {
        VARIANT::from(
            e.action
                .map(|id| format!("action-{id}"))
                .unwrap_or_else(|| "notch".into())
                .as_str(),
        )
    } else if id == UIA_ControlTypePropertyId {
        VARIANT::from(if e.action.is_some() {
            UIA_ButtonControlTypeId.0
        } else {
            UIA_PaneControlTypeId.0
        })
    } else if id == UIA_IsEnabledPropertyId {
        VARIANT::from(current.as_ref().is_none_or(|i| i.enabled))
    } else if id == UIA_IsControlElementPropertyId || id == UIA_IsContentElementPropertyId {
        VARIANT::from(true)
    } else if id == UIA_IsKeyboardFocusablePropertyId {
        VARIANT::from(current.as_ref().is_some_and(|i| i.enabled))
    } else if id == UIA_HasKeyboardFocusPropertyId {
        VARIANT::from(
            e.action
                .is_some_and(|id| e.shared.focused.load(Ordering::Relaxed) == id),
        )
    } else {
        VARIANT::default()
    };
    out.write(value);
    S_OK
}
unsafe extern "system" fn host(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = ptr::null_mut();
    let e = element(this);
    if !e.shared.active.load(Ordering::Acquire) {
        return unavailable();
    }
    if e.action.is_none() {
        if let Ok(host) = UiaHostProviderFromHwnd(e.shared.hwnd()) {
            *out = host.into_raw();
        }
    }
    S_OK
}
unsafe fn fragment_out(provider: IRawElementProviderSimple, out: *mut *mut c_void) -> HRESULT {
    match provider.cast::<IRawElementProviderFragment>() {
        Ok(fragment) => {
            *out = fragment.into_raw();
            S_OK
        }
        Err(error) => error.code(),
    }
}
unsafe extern "system" fn navigate(
    this: *mut c_void,
    direction: NavigateDirection,
    out: *mut *mut c_void,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = ptr::null_mut();
    let e = element(this);
    if !e.shared.active.load(Ordering::Acquire) {
        return unavailable();
    }
    if e.action.is_some() && direction == NavigateDirection_Parent {
        return fragment_out(simple_ref(e), out);
    }
    let items = lock_items(&e.shared);
    let target = if e.action.is_none() {
        if direction == NavigateDirection_FirstChild {
            items.first()
        } else if direction == NavigateDirection_LastChild {
            items.last()
        } else {
            None
        }
    } else {
        let Some(index) = items.iter().position(|i| Some(i.action_id) == e.action) else {
            return unavailable();
        };
        if direction == NavigateDirection_NextSibling {
            items.get(index + 1)
        } else if direction == NavigateDirection_PreviousSibling {
            index.checked_sub(1).and_then(|i| items.get(i))
        } else {
            None
        }
    }
    .map(|i| i.action_id);
    drop(items);
    if let Some(id) = target {
        fragment_out(make(e.shared.clone(), Some(id), Some(simple_ref(e))), out)
    } else {
        S_OK
    }
}
unsafe extern "system" fn runtime(this: *mut c_void, out: *mut *mut SAFEARRAY) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = ptr::null_mut();
    let e = element(this);
    if !e.shared.active.load(Ordering::Acquire) {
        return unavailable();
    }
    let Some(action) = e.action else {
        return S_OK;
    };
    let ids = [3i32, action as i32, (action >> 32) as i32]; // UiaAppendRuntimeId
    let array = SafeArrayCreateVector(VT_I4, 0, ids.len() as u32);
    if array.is_null() {
        return HRESULT(0x8007000Eu32 as i32);
    }
    for (index, id) in ids.iter().enumerate() {
        if let Err(error) = SafeArrayPutElement(array, &(index as i32), (id as *const i32).cast()) {
            let _ = SafeArrayDestroy(array);
            return error.code();
        }
    }
    *out = array;
    S_OK
}
unsafe extern "system" fn bounds(this: *mut c_void, out: *mut UiaRect) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let e = element(this);
    if !e.shared.active.load(Ordering::Acquire) || !IsWindow(Some(e.shared.hwnd())).as_bool() {
        return unavailable();
    }
    let rect = if e.action.is_some() {
        let Some(item) = item(e) else {
            return unavailable();
        };
        let mut p = POINT {
            x: item.rect.left,
            y: item.rect.top,
        };
        let _ = ClientToScreen(e.shared.hwnd(), &mut p);
        RECT {
            left: p.x,
            top: p.y,
            right: p.x + item.rect.right - item.rect.left,
            bottom: p.y + item.rect.bottom - item.rect.top,
        }
    } else {
        let mut rect = RECT::default();
        if let Err(error) = GetWindowRect(e.shared.hwnd(), &mut rect) {
            return error.code();
        }
        rect
    };
    out.write(UiaRect {
        left: rect.left as f64,
        top: rect.top as f64,
        width: (rect.right - rect.left).max(0) as f64,
        height: (rect.bottom - rect.top).max(0) as f64,
    });
    S_OK
}
unsafe extern "system" fn embedded(_: *mut c_void, out: *mut *mut SAFEARRAY) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = ptr::null_mut();
    S_OK
}
unsafe extern "system" fn focus(this: *mut c_void) -> HRESULT {
    let e = element(this);
    if let Some(id) = e.action {
        let Some(item) = item(e) else {
            return unavailable();
        };
        if !item.enabled {
            return HRESULT(UIA_E_ELEMENTNOTENABLED as i32);
        }
        e.shared.focused.store(id, Ordering::Relaxed);
        if let Err(error) = PostMessageW(
            Some(e.shared.hwnd()),
            ACCESSIBLE_FOCUS,
            WPARAM(id),
            LPARAM(0),
        ) {
            return error.code();
        }
    }
    S_OK
}
unsafe extern "system" fn fragment_root(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = ptr::null_mut();
    if !element(this).shared.active.load(Ordering::Acquire) {
        return unavailable();
    }
    match simple_ref(element(this)).cast::<IRawElementProviderFragmentRoot>() {
        Ok(root) => {
            *out = root.into_raw();
            S_OK
        }
        Err(error) => error.code(),
    }
}
unsafe extern "system" fn from_point(
    this: *mut c_void,
    x: f64,
    y: f64,
    out: *mut *mut c_void,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = ptr::null_mut();
    let e = element(this);
    if !e.shared.active.load(Ordering::Acquire) {
        return unavailable();
    }
    let mut screen_rect = RECT::default();
    if GetWindowRect(e.shared.hwnd(), &mut screen_rect).is_err() {
        return unavailable();
    }
    if x < screen_rect.left as f64
        || x >= screen_rect.right as f64
        || y < screen_rect.top as f64
        || y >= screen_rect.bottom as f64
    {
        return S_OK;
    }
    let mut origin = POINT::default();
    let _ = ClientToScreen(e.shared.hwnd(), &mut origin);
    let (x, y) = (x - origin.x as f64, y - origin.y as f64);
    let target = lock_items(&e.shared)
        .iter()
        .rev()
        .find(|item| {
            x >= item.rect.left as f64
                && x < item.rect.right as f64
                && y >= item.rect.top as f64
                && y < item.rect.bottom as f64
        })
        .map(|item| item.action_id);
    if let Some(id) = target {
        fragment_out(make(e.shared.clone(), Some(id), Some(simple_ref(e))), out)
    } else {
        fragment_out(simple_ref(e), out)
    }
}
unsafe extern "system" fn get_focus(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = ptr::null_mut();
    let e = element(this);
    if !e.shared.active.load(Ordering::Acquire) {
        return unavailable();
    }
    let id = e.shared.focused.load(Ordering::Relaxed);
    if lock_items(&e.shared).iter().any(|i| i.action_id == id) {
        fragment_out(make(e.shared.clone(), Some(id), Some(simple_ref(e))), out)
    } else {
        S_OK
    }
}
unsafe extern "system" fn invoke(this: *mut c_void) -> HRESULT {
    let e = element(this);
    let Some(item) = item(e) else {
        return unavailable();
    };
    if !item.enabled {
        return HRESULT(UIA_E_ELEMENTNOTENABLED as i32);
    }
    match PostMessageW(
        Some(e.shared.hwnd()),
        ACCESSIBLE_INVOKE,
        WPARAM(item.action_id),
        LPARAM(0),
    ) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}
static SIMPLE: IRawElementProviderSimple_Vtbl = IRawElementProviderSimple_Vtbl {
    base__: unknown(),
    ProviderOptions: options,
    GetPatternProvider: pattern,
    GetPropertyValue: property,
    HostRawElementProvider: host,
};
static FRAGMENT: IRawElementProviderFragment_Vtbl = IRawElementProviderFragment_Vtbl {
    base__: unknown(),
    Navigate: navigate,
    GetRuntimeId: runtime,
    BoundingRectangle: bounds,
    GetEmbeddedFragmentRoots: embedded,
    SetFocus: focus,
    FragmentRoot: fragment_root,
};
static ROOT: IRawElementProviderFragmentRoot_Vtbl = IRawElementProviderFragmentRoot_Vtbl {
    base__: unknown(),
    ElementProviderFromPoint: from_point,
    GetFocus: get_focus,
};
static INVOKE: IInvokeProvider_Vtbl = IInvokeProvider_Vtbl {
    base__: unknown(),
    Invoke: invoke,
};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_null_results_and_child_lifetime() {
        unsafe {
            let items = Arc::new(Mutex::new(vec![AccessibleItem {
                label: "Play".into(),
                rect: RECT::default(),
                action_id: 7,
                enabled: true,
            }]));
            let root = provider(HWND::default(), items.clone());
            let fragment = root.cast::<IRawElementProviderFragment>().unwrap();
            let child = fragment.Navigate(NavigateDirection_FirstChild).unwrap();
            let mut raw = std::ptr::dangling_mut::<c_void>();
            let result = (Interface::vtable(&child).Navigate)(
                child.as_raw(),
                NavigateDirection_NextSibling,
                &mut raw,
            );
            assert_eq!(result, S_OK);
            assert!(raw.is_null());
            let mut unknown = std::ptr::dangling_mut::<c_void>();
            assert_eq!(
                (Interface::vtable(&root).GetPatternProvider)(
                    root.as_raw(),
                    UIA_ValuePatternId,
                    &mut unknown
                ),
                S_OK
            );
            assert!(unknown.is_null());
            drop(fragment);
            drop(root);
            // A surviving child holds its original root alive.
            assert!(child.Navigate(NavigateDirection_Parent).is_ok());
            assert!(child.FragmentRoot().is_ok());
            items.lock().unwrap().clear();
            let simple = child.cast::<IRawElementProviderSimple>().unwrap();
            assert_eq!(
                simple
                    .GetPropertyValue(UIA_NamePropertyId)
                    .err()
                    .unwrap()
                    .code(),
                unavailable()
            );
        }
    }
    #[test]
    fn disabled_actions_are_not_invoked() {
        unsafe {
            let items = Arc::new(Mutex::new(vec![AccessibleItem {
                label: "Next".into(),
                rect: RECT::default(),
                action_id: 2,
                enabled: false,
            }]));
            let root = provider(HWND::default(), items);
            let child = root
                .cast::<IRawElementProviderFragment>()
                .unwrap()
                .Navigate(NavigateDirection_FirstChild)
                .unwrap();
            assert_eq!(
                child
                    .cast::<IInvokeProvider>()
                    .unwrap()
                    .Invoke()
                    .err()
                    .unwrap()
                    .code(),
                HRESULT(UIA_E_ELEMENTNOTENABLED as i32)
            );
        }
    }
    #[test]
    fn window_disconnect_invalidates_retained_children() {
        unsafe {
            let items = Arc::new(Mutex::new(vec![AccessibleItem {
                label: "Play".into(),
                rect: RECT::default(),
                action_id: 1,
                enabled: true,
            }]));
            let root = provider(HWND::default(), items);
            let fragment = root.cast::<IRawElementProviderFragment>().unwrap();
            let child = fragment.Navigate(NavigateDirection_FirstChild).unwrap();
            disconnect(&root);
            assert_eq!(
                child
                    .cast::<IInvokeProvider>()
                    .unwrap()
                    .Invoke()
                    .err()
                    .unwrap()
                    .code(),
                unavailable()
            );
            assert_eq!(
                fragment
                    .Navigate(NavigateDirection_FirstChild)
                    .err()
                    .unwrap()
                    .code(),
                unavailable()
            );
        }
    }
    #[test]
    fn help_text_property_returns_label() {
        unsafe {
            let items = Arc::new(Mutex::new(vec![AccessibleItem {
                label: "Play".into(),
                rect: RECT::default(),
                action_id: 1,
                enabled: true,
            }]));
            let root = provider(HWND::default(), items);
            let fragment = root.cast::<IRawElementProviderFragment>().unwrap();
            let child = fragment.Navigate(NavigateDirection_FirstChild).unwrap();
            let simple = child.cast::<IRawElementProviderSimple>().unwrap();
            let val = simple.GetPropertyValue(UIA_HelpTextPropertyId).unwrap();
            assert_eq!(val.to_string(), "Play");
        }
    }
}
