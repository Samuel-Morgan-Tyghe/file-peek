mod content;

use block2::{DynBlock, RcBlock};
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::*;
use objc2_foundation::{
    MainThreadMarker, NSArray, NSBundle, NSData, NSDictionary, NSError, NSNotification, NSObject,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL, NSURLResponse, ns_string,
};
use objc2_quick_look_ui::{QLPreviewItem, QLPreviewView, QLPreviewViewStyle};
use objc2_uniform_type_identifiers::UTType;
use objc2_web_kit::*;
use std::{
    cell::{Cell, OnceCell, RefCell},
    path::{Path, PathBuf},
};

thread_local! { static DELEGATE: OnceCell<Retained<Delegate>> = const { OnceCell::new() }; }

fn delegate() -> Retained<Delegate> {
    DELEGATE.with(|cell| {
        cell.get()
            .expect("Delegate is installed before event delivery")
            .clone()
    })
}

fn frame(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn url_path(url: &NSURL) -> Option<PathBuf> {
    url.isFileURL()
        .then(|| url.path().map(|path| PathBuf::from(path.to_string())))
        .flatten()
}

fn dropped_file(sender: &ProtocolObject<dyn NSDraggingInfo>) -> Option<PathBuf> {
    // SAFETY: AppKit supplies a live drag session on the main thread.
    let value = sender
        .draggingPasteboard()
        .stringForType(ns_string!("public.file-url"))?;
    url::Url::parse(&value.to_string())
        .ok()?
        .to_file_path()
        .ok()
}

// SAFETY: All AppKit subclasses and their callbacks are main-thread-only; none implements Drop.
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    struct DropView;
    unsafe impl NSObjectProtocol for DropView {}
    unsafe impl NSDraggingDestination for DropView {
        #[unsafe(method(draggingEntered:))]
        fn entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            if dropped_file(sender).is_some() {
                NSDragOperation::Copy
            } else {
                NSDragOperation::None
            }
        }
        #[unsafe(method(performDragOperation:))]
        fn dropped(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            if let Some(path) = dropped_file(sender) {
                delegate().open_path(path);
                true
            } else {
                false
            }
        }
    }
);

define_class!(
    #[unsafe(super(WKWebView))]
    #[thread_kind = MainThreadOnly]
    struct MarkdownView;
    unsafe impl NSObjectProtocol for MarkdownView {}
    unsafe impl NSDraggingDestination for MarkdownView {
        #[unsafe(method(draggingEntered:))]
        fn entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            if dropped_file(sender).is_some() {
                NSDragOperation::Copy
            } else {
                NSDragOperation::None
            }
        }
        #[unsafe(method(performDragOperation:))]
        fn dropped(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            if let Some(path) = dropped_file(sender) {
                delegate().open_path(path);
                true
            } else {
                false
            }
        }
    }
);

#[derive(Default)]
struct AssetIvars {
    directory: RefCell<Option<PathBuf>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = AssetIvars]
    struct AssetHandler;
    unsafe impl NSObjectProtocol for AssetHandler {}
    unsafe impl WKURLSchemeHandler for AssetHandler {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start(&self, _web: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            // SAFETY: Tasks are completed synchronously, before WebKit can cancel them on this thread.
            unsafe {
                let response = (|| {
                    let url = task.request().URL().ok_or("Missing image URL")?;
                    let address = url.absoluteString().ok_or("Invalid image URL")?.to_string();
                    let directory = self.ivars().directory.borrow();
                    let file = content::local_asset(
                        &address,
                        directory.as_deref().ok_or("No document open")?,
                    )
                    .ok_or("Image is outside the document folder or unavailable")?;
                    let kind = UTType::typeWithFilenameExtension(&NSString::from_str(
                        &content::extension(&file),
                    ))
                    .ok_or("Unknown image type")?;
                    let image_type = UTType::typeWithIdentifier(ns_string!("public.image"))
                        .ok_or("Image type unavailable")?;
                    if !kind.conformsToType(&image_type) {
                        return Err("Only image assets can be loaded".into());
                    }
                    let mime = kind.preferredMIMEType().ok_or("Unknown image format")?;
                    Ok::<_, String>((
                        url,
                        mime,
                        content::read_limited(&file, content::MAX_IMAGE_BYTES)?,
                    ))
                })();
                match response {
                    Ok((url, mime, bytes)) => {
                        let response = NSURLResponse::initWithURL_MIMEType_expectedContentLength_textEncodingName(NSURLResponse::alloc(), &url, Some(&mime), bytes.len() as isize, None);
                        task.didReceiveResponse(&response);
                        task.didReceiveData(&NSData::with_bytes(&bytes));
                        task.didFinish();
                    }
                    Err(_) => task.didFailWithError(&NSError::errorWithDomain_code_userInfo(
                        ns_string!("FilePeek.Asset"),
                        1,
                        None,
                    )),
                }
            }
        }
        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop(&self, _web: &WKWebView, _task: &ProtocolObject<dyn WKURLSchemeTask>) {}
    }
);

struct Document {
    path: PathBuf,
    text: Option<String>,
    html: String,
    size: u64,
}

struct Ui {
    window: Retained<NSWindow>,
    web: Retained<MarkdownView>,
    quick: Retained<QLPreviewView>,
    text: Retained<NSTextView>,
    source: Retained<NSScrollView>,
    empty: Retained<NSView>,
    footer: Retained<NSTextField>,
    mode: Retained<NSSegmentedControl>,
    assets: Retained<AssetHandler>,
    resources: PathBuf,
}

#[derive(Default)]
struct DelegateIvars {
    ui: OnceCell<Ui>,
    document: RefCell<Option<Document>>,
    request_id: Cell<u64>,
    web_ready: Cell<bool>,
    test_started: Cell<bool>,
    self_test: bool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSWindowDelegate for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn launched(&self, _notification: &NSNotification) {
            self.ensure_ui();
            if self.ivars().self_test {
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_secs(20));
                    eprintln!("Renderer self-test timed out");
                    std::process::exit(1);
                });
                self.open_path(PathBuf::from("Tests/fixture.md"));
            } else {
                self.ui().window.makeKeyAndOrderFront(None);
                #[allow(deprecated)]
                NSApplication::sharedApplication(self.mtm()).activateIgnoringOtherApps(true);
                if self.ivars().request_id.get() == 0 {
                    if let Some(path) = std::env::args().nth(1).filter(|path| !path.starts_with('-')) { self.open_path(path.into()); }
                }
            }
        }

        #[unsafe(method(application:openFiles:))]
        fn open_files(&self, app: &NSApplication, files: &NSArray<NSString>) {
            self.ensure_ui();
            if let Some(path) = files.firstObject() { self.open_path(PathBuf::from(path.to_string())); }
            app.replyToOpenOrPrint(NSApplicationDelegateReply::Success);
        }

        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn reopen(&self, _app: &NSApplication, _visible: bool) -> bool {
            self.ensure_ui();
            self.ui().window.makeKeyAndOrderFront(None);
            true
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn terminating(&self, _notification: &NSNotification) {
            if let Some(ui) = self.ivars().ui.get() {
                // SAFETY: The retained preview is closed exactly once, on app termination.
                unsafe { ui.quick.close(); }
            }
        }
    }

    unsafe impl WKNavigationDelegate for Delegate {
        #[unsafe(method(webView:didFinishNavigation:))]
        fn loaded(&self, _web: &WKWebView, _navigation: Option<&WKNavigation>) {
            self.ivars().web_ready.set(true);
            self.render();
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn navigation_failed(&self, _web: &WKWebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            self.show_error(&error.localizedDescription().to_string());
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn provisional_failed(&self, _web: &WKWebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            self.show_error(&error.localizedDescription().to_string());
        }

        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn navigation(&self, _web: &WKWebView, action: &WKNavigationAction, reply: &DynBlock<dyn Fn(WKNavigationActionPolicy)>) {
            // SAFETY: WebKit supplies the action and synchronous decision block on the main thread.
            unsafe {
                let Some(url) = action.request().URL() else { reply.call((WKNavigationActionPolicy::Cancel,)); return; };
                if action.navigationType() == WKNavigationType::LinkActivated {
                    reply.call((WKNavigationActionPolicy::Cancel,));
                    let scheme = url.scheme().map(|value| value.to_string()).unwrap_or_default();
                    if ["https", "http", "mailto"].contains(&scheme.as_str()) { NSWorkspace::sharedWorkspace().openURL(&url); }
                    else {
                        let path = self.ivars().document.borrow().as_ref().and_then(|doc| {
                            content::local_asset(&url.absoluteString()?.to_string(), doc.path.parent()?)
                        });
                        if let Some(path) = path { self.open_path(path); }
                    }
                } else {
                    let allowed = url_path(&url).is_some_and(|path| path.starts_with(&self.ui().resources));
                    reply.call((if allowed { WKNavigationActionPolicy::Allow } else { WKNavigationActionPolicy::Cancel },));
                }
            }
        }
    }

    unsafe impl NSToolbarDelegate for Delegate {
        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_items(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            toolbar_ids()
        }
        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_items(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> { toolbar_ids() }
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn toolbar_item(&self, _toolbar: &NSToolbar, id: &NSString, _insert: bool) -> Option<Retained<NSToolbarItem>> {
            self.make_toolbar_item(id)
        }
    }

    impl Delegate {
        #[unsafe(method(chooseFile:))]
        fn choose_file(&self, _sender: Option<&AnyObject>) {
            let panel = NSOpenPanel::openPanel(self.mtm());
            {
                panel.setCanChooseDirectories(false);
                panel.setAllowsMultipleSelection(false);
                panel.setPrompt(Some(ns_string!("Open")));
                let captured = panel.clone();
                let callback = RcBlock::new(move |response: NSModalResponse| {
                    if response == NSModalResponseOK {
                        if let Some(path) = captured.URL().as_deref().and_then(url_path) { delegate().open_path(path); }
                    }
                });
                panel.beginSheetModalForWindow_completionHandler(&self.ui().window, &callback);
            }
        }
        #[unsafe(method(reload:))]
        fn reload(&self, _sender: Option<&AnyObject>) {
            let path = self.ivars().document.borrow().as_ref().map(|doc| doc.path.clone());
            if let Some(path) = path { self.open_path(path); }
        }
        #[unsafe(method(reveal:))]
        fn reveal(&self, _sender: Option<&AnyObject>) {
            if let Some(doc) = self.ivars().document.borrow().as_ref() {
                NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_slice(&[&*file_url(&doc.path)]));
            }
        }
        #[unsafe(method(closeWindow:))]
        fn close_window(&self, _sender: Option<&AnyObject>) { self.ui().window.close(); }
        #[unsafe(method(changeMode:))]
        fn change_mode(&self, _sender: Option<&AnyObject>) {
            self.apply_mode();
        }
        #[unsafe(method(toggleSource:))]
        fn toggle_source(&self, _sender: Option<&AnyObject>) {
            let ui = self.ui();
            if ui.mode.isEnabled() { ui.mode.setSelectedSegment(1 - ui.mode.selectedSegment()); self.apply_mode(); }
        }
        #[unsafe(method(zoomIn:))]
        fn zoom_in(&self, _sender: Option<&AnyObject>) { self.zoom(0.1); }
        #[unsafe(method(zoomOut:))]
        fn zoom_out(&self, _sender: Option<&AnyObject>) { self.zoom(-0.1); }
        #[unsafe(method(actualSize:))]
        fn actual_size(&self, _sender: Option<&AnyObject>) {
            // SAFETY: UI objects are retained and only accessed on the main thread.
            unsafe { self.ui().web.setPageZoom(1.0); self.ui().text.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(13.0, 0.0))); }
        }
    }
);

impl Delegate {
    fn apply_mode(&self) {
        let ui = self.ui();
        if ui.mode.isEnabled() {
            self.display(if ui.mode.selectedSegment() == 0 {
                &ui.web
            } else {
                &ui.source
            });
        }
    }

    fn make_toolbar_item(&self, id: &NSString) -> Option<Retained<NSToolbarItem>> {
        // SAFETY: Targets live for the app lifetime and each selector below is defined on Delegate.
        unsafe {
            let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(self.mtm()), id);
            if id.to_string() == "mode" {
                item.setView(Some(&self.ui().mode));
                item.setLabel(ns_string!("View"));
                return Some(item);
            }
            let (label, symbol, action) = match id.to_string().as_str() {
                "open" => ("Open File", "folder", sel!(chooseFile:)),
                "reload" => ("Reload", "arrow.clockwise", sel!(reload:)),
                "finder" => (
                    "Show in Finder",
                    "folder.badge.magnifyingglass",
                    sel!(reveal:),
                ),
                _ => return None,
            };
            item.setLabel(&NSString::from_str(label));
            item.setToolTip(Some(&NSString::from_str(label)));
            item.setImage(
                NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(symbol),
                    Some(&NSString::from_str(label)),
                )
                .as_deref(),
            );
            item.setTarget(Some(self));
            item.setAction(Some(action));
            Some(item)
        }
    }

    fn new(mtm: MainThreadMarker, self_test: bool) -> Retained<Self> {
        let allocated = Self::alloc(mtm).set_ivars(DelegateIvars {
            self_test,
            ..Default::default()
        });
        // SAFETY: NSObject initialization is called once after initializing the Rust ivars.
        unsafe { msg_send![super(allocated), init] }
    }

    fn ui(&self) -> &Ui {
        self.ivars()
            .ui
            .get()
            .expect("UI is initialized before handling actions")
    }

    fn ensure_ui(&self) {
        if self.ivars().ui.get().is_some() {
            return;
        }
        match Ui::new(self) {
            Ok(ui) => {
                let _ = self.ivars().ui.set(ui);
                self.create_menu();
                // SAFETY: Toolbar and its delegate stay retained throughout the app lifetime.
                unsafe {
                    let toolbar = NSToolbar::initWithIdentifier(
                        NSToolbar::alloc(self.mtm()),
                        ns_string!("FilePeekToolbar"),
                    );
                    toolbar.setDelegate(Some(ProtocolObject::from_ref(self)));
                    toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
                    self.ui().window.setToolbar(Some(&toolbar));
                    self.ui()
                        .window
                        .setToolbarStyle(NSWindowToolbarStyle::Unified);
                    let html = self.ui().resources.join("preview.html");
                    self.ui().web.loadFileURL_allowingReadAccessToURL(
                        &file_url(&html),
                        &file_url(&self.ui().resources),
                    );
                }
            }
            Err(error) => {
                eprintln!("File Peek couldn't start: {error}");
                std::process::exit(1);
            }
        }
    }

    fn open_path(&self, path: PathBuf) {
        self.ensure_ui();
        let id = self.ivars().request_id.get().wrapping_add(1);
        self.ivars().request_id.set(id);
        if !self.ivars().self_test {
            self.ui().window.makeKeyAndOrderFront(None);
            #[allow(deprecated)]
            NSApplication::sharedApplication(self.mtm()).activateIgnoringOtherApps(true);
        }
        let is_text = content::known_text(&path) || native_text_type(&path);
        self.ui()
            .footer
            .setStringValue(&NSString::from_str(&format!(
                "Opening {}…",
                path.file_name().unwrap_or_default().to_string_lossy()
            )));
        std::thread::spawn(move || {
            let result = (|| -> Result<Document, String> {
                let path = path.canonicalize().map_err(|error| error.to_string())?;
                let info = path.metadata().map_err(|error| error.to_string())?;
                if !info.is_file() {
                    return Err("Choose a file to preview, rather than a folder.".into());
                }
                let text = if is_text {
                    Some(content::decode_text(&content::read_limited(
                        &path,
                        content::MAX_TEXT_BYTES,
                    )?)?)
                } else {
                    None
                };
                let html = if content::is_markdown(&path) {
                    content::markdown_html(text.as_deref().unwrap_or(""))
                } else {
                    String::new()
                };
                Ok(Document {
                    path,
                    text,
                    html,
                    size: info.len(),
                })
            })();
            DispatchQueue::main().exec_async(move || {
                let app = delegate();
                if app.ivars().request_id.get() != id {
                    return;
                }
                match result {
                    Ok(doc) => app.present(doc),
                    Err(error) => app.show_error(&error),
                }
            });
        });
    }

    fn present(&self, document: Document) {
        let ui = self.ui();
        let path = &document.path;
        let markdown = content::is_markdown(path);
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        ui.window.setTitle(&NSString::from_str(&name));
        ui.window.setRepresentedURL(Some(&file_url(path)));
        let parent = NSString::from_str(&path.parent().unwrap_or(Path::new("/")).to_string_lossy());
        ui.window
            .setSubtitle(&parent.stringByAbbreviatingWithTildeInPath());
        let kind = content::extension(path).to_uppercase();
        let size = if document.size < 1024 {
            format!("{} bytes", document.size)
        } else {
            format!("{:.1} KB", document.size as f64 / 1024.0)
        };
        ui.footer.setStringValue(&NSString::from_str(&format!(
            "{}  ·  {}  ·  Read only",
            if kind.is_empty() { "FILE" } else { &kind },
            size
        )));
        // SAFETY: The NSURL and text values are retained/copied by their AppKit properties.
        unsafe {
            ui.footer
                .setToolTip(Some(&NSString::from_str(&path.to_string_lossy())));
            ui.mode.setEnabled(markdown);
            ui.mode.setSelectedSegment(0);
            ui.quick.setPreviewItem(None);
            *ui.assets.ivars().directory.borrow_mut() = path.parent().map(Path::to_path_buf);
            if let Some(text) = &document.text {
                ui.text.setString(&NSString::from_str(text));
                ui.text.scrollToBeginningOfDocument(None);
                self.display(if markdown { &ui.web } else { &ui.source });
            } else {
                ui.quick
                    .setPreviewItem(Some(ProtocolObject::<dyn QLPreviewItem>::from_ref(
                        &*file_url(path),
                    )));
                self.display(&ui.quick);
            }
            NSDocumentController::sharedDocumentController(self.mtm())
                .noteNewRecentDocumentURL(&file_url(path));
        }
        self.ivars().document.replace(Some(document));
        if markdown {
            self.render();
        }
    }

    fn display(&self, selected: &NSView) {
        let ui = self.ui();
        for view in [&*ui.web as &NSView, &*ui.quick, &*ui.source, &*ui.empty] {
            view.setHidden(!std::ptr::eq(view, selected));
        }
    }

    fn render(&self) {
        if !self.ivars().web_ready.get() {
            return;
        }
        let html = self
            .ivars()
            .document
            .borrow()
            .as_ref()
            .filter(|doc| content::is_markdown(&doc.path))
            .map(|doc| doc.html.clone());
        let Some(html) = html else {
            return;
        };
        let html = NSString::from_str(&html);
        let arguments =
            NSDictionary::<NSString, AnyObject>::from_slices(&[ns_string!("html")], &[&*html]);
        let id = self.ivars().request_id.get();
        let testing = self.ivars().self_test;
        if testing && self.ivars().test_started.replace(true) {
            return;
        }
        let body = if testing {
            include_str!("../Tests/renderer.test.js")
        } else {
            "return renderHTML(html)"
        };
        let callback = RcBlock::new(move |_value: *mut AnyObject, error: *mut NSError| {
            // SAFETY: WebKit guarantees callback pointers are either null or valid for the callback duration.
            if let Some(error) = unsafe { error.as_ref() } {
                delegate().show_error(&error.localizedDescription().to_string());
            } else if testing {
                println!(
                    "Renderer tests passed: Rust parsing, native file loading, tables, tasks, code, Unicode, anchors, sanitization and scoped image loading"
                );
                std::process::exit(0);
            } else if delegate().ivars().request_id.get() != id {
                delegate().render();
            }
        });
        // SAFETY: Arguments are passed as data, not executable source; WebKit copies the retained callback.
        unsafe {
            self.ui()
                .web
                .callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                    &NSString::from_str(body),
                    Some(&arguments),
                    None,
                    &WKContentWorld::pageWorld(self.mtm()),
                    Some(&callback),
                );
        }
    }

    fn show_error(&self, message: &str) {
        if self.ivars().self_test {
            eprintln!("Self-test failed: {message}");
            std::process::exit(1);
        }
        let ui = self.ui();
        ui.footer.setStringValue(ns_string!("Couldn't open file"));
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("Couldn't preview this file"));
        alert.setInformativeText(&NSString::from_str(message));
        // SAFETY: The window is retained until termination; this sheet has no completion state.
        alert.beginSheetModalForWindow_completionHandler(&ui.window, None);
    }

    fn zoom(&self, amount: f64) {
        // SAFETY: Both views are owned by this main-thread delegate.
        unsafe {
            let scale = (self.ui().web.pageZoom() + amount).clamp(0.6, 2.5);
            self.ui().web.setPageZoom(scale);
            self.ui()
                .text
                .setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
                    13.0 * scale,
                    0.0,
                )));
        }
    }

    fn create_menu(&self) {
        let mtm = self.mtm();
        let menu = NSMenu::new(mtm);
        // SAFETY: Menu selectors either target this retained delegate or standard AppKit responders.
        unsafe {
            for (title, entries) in [
                (
                    "File Peek",
                    vec![
                        (
                            "About File Peek",
                            sel!(orderFrontStandardAboutPanel:),
                            "",
                            false,
                        ),
                        ("Quit File Peek", sel!(terminate:), "q", false),
                    ],
                ),
                (
                    "File",
                    vec![
                        ("Open…", sel!(chooseFile:), "o", true),
                        ("Reload", sel!(reload:), "r", true),
                        ("Show in Finder", sel!(reveal:), "", true),
                        ("Close Window", sel!(closeWindow:), "w", true),
                    ],
                ),
                (
                    "Edit",
                    vec![
                        ("Copy", sel!(copy:), "c", false),
                        ("Select All", sel!(selectAll:), "a", false),
                    ],
                ),
                (
                    "View",
                    vec![
                        ("Preview / Source", sel!(toggleSource:), "e", true),
                        ("Zoom In", sel!(zoomIn:), "+", true),
                        ("Zoom Out", sel!(zoomOut:), "-", true),
                        ("Actual Size", sel!(actualSize:), "0", true),
                    ],
                ),
            ] {
                let top = NSMenuItem::new(mtm);
                let sub = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
                for (label, action, key, own) in entries {
                    let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(label),
                        Some(action),
                        &NSString::from_str(key),
                    );
                    if own {
                        item.setTarget(Some(self));
                    }
                    sub.addItem(&item);
                }
                top.setSubmenu(Some(&sub));
                menu.addItem(&top);
            }
        }
        NSApplication::sharedApplication(mtm).setMainMenu(Some(&menu));
    }
}

fn native_text_type(path: &Path) -> bool {
    let Some(kind) =
        UTType::typeWithFilenameExtension(&NSString::from_str(&content::extension(path)))
    else {
        return false;
    };
    let Some(text) = UTType::typeWithIdentifier(ns_string!("public.text")) else {
        return false;
    };
    kind.conformsToType(&text)
}

fn toolbar_ids() -> Retained<NSArray<NSString>> {
    NSArray::from_slice(&[
        ns_string!("open"),
        ns_string!("NSToolbarFlexibleSpaceItem"),
        ns_string!("mode"),
        ns_string!("NSToolbarFlexibleSpaceItem"),
        ns_string!("reload"),
        ns_string!("finder"),
    ])
}

impl Ui {
    fn new(owner: &Delegate) -> Result<Self, String> {
        let mtm = owner.mtm();
        let resources = NSBundle::mainBundle()
            .resourceURL()
            .as_deref()
            .and_then(url_path)
            .ok_or("Missing app resources")?;
        if !resources.join("preview.html").is_file() {
            return Err("Run the bundled File Peek.app produced by build.sh.".into());
        }
        // SAFETY: UI creation stays on the main thread; NSWindow automatic release is disabled,
        // all delegate/target objects are retained for the entire application lifetime.
        unsafe {
            let window = NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame(0.0, 0.0, 980.0, 740.0),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            );
            window.setReleasedWhenClosed(false);
            window.setDelegate(Some(ProtocolObject::from_ref(owner)));
            window.setTitle(ns_string!("File Peek"));
            window.setContentMinSize(NSSize::new(520.0, 360.0));
            window.setFrameAutosaveName(ns_string!("FilePeekWindow"));
            window.center();
            let root = window.contentView().ok_or("Missing window content view")?;
            let bounds = root.bounds();
            let host: Retained<DropView> = msg_send![DropView::alloc(mtm), initWithFrame: frame(0.0, 28.0, bounds.size.width, bounds.size.height - 28.0)];
            host.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            host.registerForDraggedTypes(&NSArray::from_slice(&[ns_string!("public.file-url")]));
            root.addSubview(&host);
            let footer = NSTextField::labelWithString(
                ns_string!("Markdown, text, PDFs, images and more"),
                mtm,
            );
            footer.setFrame(frame(16.0, 7.0, bounds.size.width - 32.0, 15.0));
            footer.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            footer.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            footer.setTextColor(Some(&NSColor::secondaryLabelColor()));
            root.addSubview(&footer);

            let allocated = AssetHandler::alloc(mtm).set_ivars(AssetIvars::default());
            let assets: Retained<AssetHandler> = msg_send![super(allocated), init];
            let config = WKWebViewConfiguration::new(mtm);
            config.setWebsiteDataStore(&WKWebsiteDataStore::nonPersistentDataStore(mtm));
            config.setURLSchemeHandler_forURLScheme(
                Some(ProtocolObject::from_ref(&*assets)),
                ns_string!("peek-asset"),
            );
            let web: Retained<MarkdownView> = msg_send![MarkdownView::alloc(mtm), initWithFrame: host.bounds(), configuration: &*config];
            web.setNavigationDelegate(Some(ProtocolObject::from_ref(owner)));
            web.setAllowsBackForwardNavigationGestures(false);
            web.registerForDraggedTypes(&NSArray::from_slice(&[ns_string!("public.file-url")]));
            let quick = QLPreviewView::initWithFrame_style(
                QLPreviewView::alloc(mtm),
                host.bounds(),
                QLPreviewViewStyle::Normal,
            )
            .ok_or("Quick Look is unavailable")?;
            quick.setShouldCloseWithWindow(false);
            quick.setAutostarts(false);
            let text = NSTextView::initWithFrame(NSTextView::alloc(mtm), host.bounds());
            text.setEditable(false);
            text.setSelectable(true);
            text.setRichText(false);
            text.unregisterDraggedTypes();
            text.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(13.0, 0.0)));
            text.setTextContainerInset(NSSize::new(24.0, 24.0));
            text.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            text.setVerticallyResizable(true);
            if let Some(container) = text.textContainer() {
                container.setWidthTracksTextView(true);
            }
            let source = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), host.bounds());
            source.setDocumentView(Some(&text));
            source.setHasVerticalScroller(true);
            source.setAutohidesScrollers(true);
            let empty = NSView::initWithFrame(NSView::alloc(mtm), host.bounds());
            let heading =
                NSTextField::labelWithString(ns_string!("A quick look, beautifully clear."), mtm);
            heading.setFont(Some(&NSFont::systemFontOfSize_weight(24.0, 0.6)));
            let detail = NSTextField::labelWithString(
                ns_string!("Drop a file here, or open one to take a peek."),
                mtm,
            );
            detail.setFont(Some(&NSFont::systemFontOfSize(14.0)));
            detail.setTextColor(Some(&NSColor::secondaryLabelColor()));
            let button = NSButton::buttonWithTitle_target_action(
                ns_string!("Open File…"),
                Some(owner),
                Some(sel!(chooseFile:)),
                mtm,
            );
            let hint = NSTextField::labelWithString(
                ns_string!("⌘O to open  ·  Markdown, text, PDFs & images"),
                mtm,
            );
            hint.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            hint.setTextColor(Some(&NSColor::tertiaryLabelColor()));
            let icon = NSImageView::new(mtm);
            if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                ns_string!("doc.text.magnifyingglass"),
                None,
            ) {
                image.setSize(NSSize::new(62.0, 72.0));
                icon.setImage(Some(&image));
            }
            icon.setContentTintColor(Some(&NSColor::controlAccentColor()));
            let views: [&NSView; 5] = [&icon, &heading, &detail, &button, &hint];
            let stack = NSStackView::stackViewWithViews(&NSArray::from_slice(&views), mtm);
            stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
            stack.setAlignment(NSLayoutAttribute::CenterX);
            stack.setSpacing(16.0);
            stack.setFrame(frame(
                (host.bounds().size.width - 500.0) / 2.0,
                (host.bounds().size.height - 310.0) / 2.0,
                500.0,
                310.0,
            ));
            stack.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewMinXMargin
                    | NSAutoresizingMaskOptions::ViewMaxXMargin
                    | NSAutoresizingMaskOptions::ViewMinYMargin
                    | NSAutoresizingMaskOptions::ViewMaxYMargin,
            );
            empty.addSubview(&stack);
            for view in [&*web as &NSView, &*quick, &*source, &*empty] {
                view.setFrame(host.bounds());
                view.setAutoresizingMask(
                    NSAutoresizingMaskOptions::ViewWidthSizable
                        | NSAutoresizingMaskOptions::ViewHeightSizable,
                );
                host.addSubview(view);
                view.setHidden(!std::ptr::eq(view, &*empty));
            }
            let mode = NSSegmentedControl::segmentedControlWithLabels_trackingMode_target_action(
                &NSArray::from_slice(&[ns_string!("Preview"), ns_string!("Source")]),
                NSSegmentSwitchTracking::SelectOne,
                Some(owner),
                Some(sel!(changeMode:)),
                mtm,
            );
            mode.setSelectedSegment(0);
            mode.setEnabled(false);
            Ok(Self {
                window,
                web,
                quick,
                text,
                source,
                empty,
                footer,
                mode,
                assets,
                resources,
            })
        }
    }
}

fn main() {
    let mtm = MainThreadMarker::new().expect("Mac apps must launch on the main thread");
    let self_test = std::env::args().any(|arg| arg == "--self-test");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(if self_test {
        NSApplicationActivationPolicy::Prohibited
    } else {
        NSApplicationActivationPolicy::Regular
    });
    let owner = Delegate::new(mtm, self_test);
    DELEGATE.with(|cell| {
        let _ = cell.set(owner.clone());
    });
    app.setDelegate(Some(ProtocolObject::from_ref(&*owner)));
    app.run();
}
