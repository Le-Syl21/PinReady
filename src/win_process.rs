//! Windows: helper programs started without a console window.
//!
//! PinReady is a GUI program, so Windows gives each console program it
//! starts (PowerShell) a console of its own, which pops up on screen for as
//! long as the child runs. `CREATE_NO_WINDOW` starts it with none.

use std::os::windows::process::CommandExt;
use std::process::Command;

/// `CREATE_NO_WINDOW` process creation flag.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `powershell`, run without a console window.
pub fn powershell() -> Command {
    let mut cmd = Command::new("powershell");
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}
