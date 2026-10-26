//! Native macOS printing through AppKit.
//!
//! GPUI has no print API, so the document is handed to an `NSTextView` and
//! printed with `NSPrintOperation`. AppKit owns pagination, line wrapping and
//! the print sheet, which is the same one every other macOS app shows: printer
//! selection, presets and PDF export all come for free.

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSFont, NSPrintInfo, NSPrintOperation, NSTextView};
use objc2_foundation::{NSPoint, NSRect, NSString};

/// Point size of the fixed-pitch font the printed document uses.
const PRINT_FONT_SIZE: f64 = 11.;

/// Opens the native print sheet for `text`, titled `title`.
///
/// The panel runs its own modal event loop, so this only queues the work and
/// returns. Opening it straight from a command handler would run that loop
/// inside GPUI's update, and any GPUI task the loop picks up from the main
/// queue would then re-enter the app while its state is still mutably borrowed
/// and panic. The main queue runs the operation once the update has returned.
pub(crate) fn print_text(text: &str, title: &str) {
    let text = text.to_owned();
    let title = title.to_owned();
    DispatchQueue::main().exec_async(move || run(&text, &title));
}

/// Shows the print panel and blocks until it closes. Runs on the main thread.
fn run(text: &str, title: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };

    autoreleasepool(|_| {
        let print_info = NSPrintInfo::sharedPrintInfo();
        // Lay the text out in the imageable area of the page. It already
        // accounts for the user's paper size, orientation and margins, so the
        // view prints one page's worth at a time and NSTextView paginates the
        // text to the width it is given.
        let frame = NSRect::new(NSPoint::ZERO, print_info.imageablePageBounds().size);

        let view = NSTextView::initWithFrame(mtm.alloc::<NSTextView>(), frame);
        view.setEditable(false);
        view.setSelectable(false);
        view.setRichText(false);
        // The canonical print setup: wrap to the page width and let the view
        // grow along the text so pagination stays predictable.
        view.setVerticallyResizable(true);
        view.setHorizontallyResizable(false);
        if let Some(font) = NSFont::userFixedPitchFontOfSize(PRINT_FONT_SIZE) {
            view.setFont(Some(&font));
        }
        view.setString(&NSString::from_str(text));

        let operation = NSPrintOperation::printOperationWithView_printInfo(&view, &print_info);
        operation.setJobTitle(Some(&NSString::from_str(title)));
        operation.setShowsPrintPanel(true);
        operation.setShowsProgressPanel(true);
        operation.runOperation();
    });
}
