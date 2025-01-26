use std::sync::Arc;
use tokio::sync::Mutex;

/// Trait that provides a mechanism to stop an ongoing process.
pub trait Stoppable {
    /// Returns a reference to the stop flag, which is a shared, mutable boolean.
    fn stop_flag(&self) -> &Arc<Mutex<bool>>;

    /// Checks if the stop flag is set to true, indicating the process should stop.
    async fn should_stop(&self) -> bool {
        *self.stop_flag().lock().await
    }

    /// Sets the stop flag to the given value, controlling the stop state of the process.
    async fn set_stop_flag(&self, value: bool) {
        let mut flag = self.stop_flag().lock().await;
        *flag = value;
    }
}

/// Macro to implement the Stoppable trait for a given type.
/// The type must have a field named `stop_flag` of type `Arc<Mutex<bool>>`.
#[macro_export]
macro_rules! impl_stoppable {
    ($type:ty) => {
        impl Stoppable for $type {
            fn stop_flag(&self) -> &Arc<Mutex<bool>> {
                &self.stop_flag
            }
        }
    };
}
