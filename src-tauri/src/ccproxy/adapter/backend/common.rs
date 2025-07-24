use std::sync::RwLockWriteGuard;

use crate::ccproxy::adapter::unified::SseStatus;

pub fn update_message_block(mut status: RwLockWriteGuard<'_, SseStatus>, block: String) {
    if !status.current_content_block.is_empty() && status.current_content_block != block {
        status.message_index += 1;
    }
    status.current_content_block = block;
}
