//! Output targets. A frame maps zone index (position in config `[[zone]]`) → one RGB per LED.
pub mod openrgb;
pub mod terminal;

pub type Rgb = (u8, u8, u8);
pub type Frame = std::collections::BTreeMap<usize, Vec<Rgb>>;

pub trait Sink: Send {
    /// LED count per configured zone (0 = zone not found; skip it).
    fn sizes(&self) -> &[usize];
    fn show(&mut self, frame: &Frame, status: &str) -> anyhow::Result<()>;
    fn close(&mut self) {}
}
