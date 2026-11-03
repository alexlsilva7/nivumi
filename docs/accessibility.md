# Custom canvas accessibility

`accessibility::provider(hwnd, items)` returns the root `IRawElementProviderSimple`. Cache it while the window exists. Respond to `WM_GETOBJECT` with `UiaReturnRawElementProvider(hwnd,wparam,lparam,&provider)` when `lparam.0 == UiaRootObjectId as isize`.

Publish visible actionable regions as `AccessibleItem { label, rect, action_id, enabled }` into the shared `Arc<Mutex<Vec<_>>>`. Rectangles are client physical pixels; the provider converts to desktop coordinates. Labels must describe the current action, such as Pause, not just its icon. IDs must remain stable for the same semantic action. The provider exposes a Pane root and Button children, enabled/focus state, names, automation IDs, navigation, hit testing, screen rectangles and Invoke pattern.

Call `disconnect(&provider)` during WM_DESTROY, before releasing the cached root. This invalidates external child references and prevents posting commands to an HWND reused by another window.

Handle `ACCESSIBLE_INVOKE` (`WM_APP+50`) by dispatching the WPARAM action ID through the same command path as a mouse click. Handle `ACCESSIBLE_FOCUS` (`WM_APP+51`) by moving the canvas focus to that ID and invalidating the focus ring. After normal keyboard navigation, call `keyboard_focus(&provider, Some(action_id))` (None when focus leaves). Call `notify_changed(&provider)` when page/list actions change, never while holding the items mutex. Data-only text/lyrics are not yet individual TextPattern elements.

UIA worker calls read mutex-protected snapshots; commands are posted asynchronously to the UI thread. A child holds a COM reference to the stable root. Removed actions report UIA_E_ELEMENTNOTAVAILABLE. Missing patterns/navigation correctly return S_OK with null pointers through manual COM vtables, avoiding invalid null Rust Interface values. Reference counts are atomic. The module does not synthesize user input or invoke actions synchronously from a UIA callback.

Tests cover missing pattern/sibling null results, retained child/root lifetime, stale action behavior and disabled invocation. Narrator/Inspect interaction still requires an actual running window and should be marked unverified until exercised.
