use anyhow::{Result, bail};
use block2::RcBlock;
use objc2::{
    class, msg_send,
    runtime::{AnyObject, Bool},
};
use std::sync::mpsc;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeAudio: &'static AnyObject;
}

pub(super) const DENIED: &str =
    "无法访问麦克风，请在「系统设置 → 隐私与安全性 → 麦克风」允许声剪访问";

pub(super) fn request() -> Result<Option<mpsc::Receiver<bool>>> {
    // AVMediaTypeAudio is a framework-owned NSString constant. The system copies
    // the completion block and invokes it once with an Objective-C BOOL.
    unsafe {
        let device = class!(AVCaptureDevice);
        let status: isize = msg_send![device, authorizationStatusForMediaType: AVMediaTypeAudio];
        match status {
            3 => Ok(None), // AVAuthorizationStatusAuthorized
            0 => {
                // AVAuthorizationStatusNotDetermined
                let (tx, rx) = mpsc::channel();
                let callback = RcBlock::new(move |granted: Bool| {
                    let _ = tx.send(granted.as_bool());
                });
                let _: () = msg_send![device, requestAccessForMediaType: AVMediaTypeAudio, completionHandler: &*callback];
                Ok(Some(rx))
            }
            _ => bail!(DENIED),
        }
    }
}
