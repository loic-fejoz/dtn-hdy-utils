//! Persistent Chat storage manager (re-exports from `internal_storage`).

pub use crate::internal_storage::{
    ChatMessage, ConversationSummary, InternalStorage as ChatManager, extract_callsign,
    normalize_chat_endpoint, normalize_chat_target, normalize_peer_node,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalization() {
        assert_eq!(normalize_peer_node("f4jxq"), "dtn://f4jxq/");
        assert_eq!(normalize_peer_node("dtn://f4jxq/chat"), "dtn://f4jxq/");
        assert_eq!(normalize_peer_node("dtn://f4jxq"), "dtn://f4jxq/");
        assert_eq!(normalize_peer_node("ipn:42.1"), "ipn:42.0");

        assert_eq!(
            normalize_chat_target("dtn://f4jxq-9/chat"),
            "dtn://f4jxq-9/chat"
        );
        assert_eq!(
            normalize_chat_target("dtn://f4jxq/stats"),
            "dtn://f4jxq/stats"
        );
        assert_eq!(normalize_chat_target("dtn://f4jxq-9"), "dtn://f4jxq-9/chat");
        assert_eq!(
            normalize_chat_target("dtn://f4jxq-9/"),
            "dtn://f4jxq-9/chat"
        );
        assert_eq!(normalize_chat_target("f4jxq-9"), "dtn://f4jxq-9/chat");
        assert_eq!(normalize_chat_target("f4jxq"), "dtn://f4jxq/chat");
    }

    #[test]
    fn test_chat_manager_crud() {
        let manager = ChatManager::new_in_memory().unwrap();

        let msg = ChatMessage {
            id: "msg-1".to_string(),
            conversation_peer: "dtn://f4jxq/chat".to_string(),
            sender_eid: "dtn://f4jxq/chat".to_string(),
            recipient_eid: "dtn://stata/chat".to_string(),
            content: "Hello from F4JXQ".to_string(),
            direction: "inbound".to_string(),
            status: "received".to_string(),
            is_read: false,
            is_sticky: false,
            timestamp: 1000,
            expiry_timestamp: 86401000,
            created_at: "2026-11-04T12:00:00Z".to_string(),
            auth_status: "unsigned".to_string(),
        };

        manager.insert_message(&msg).unwrap();

        let convs = manager.list_conversations().unwrap();
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].peer, "dtn://f4jxq/chat");
        assert_eq!(convs[0].unread_count, 1);
        assert_eq!(convs[0].total_messages, 1);

        let msgs = manager.get_conversation_messages("f4jxq").unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, "msg-1");
        assert_eq!(msgs[0].content, "Hello from F4JXQ");

        let marked = manager.mark_conversation_read("f4jxq").unwrap();
        assert_eq!(marked, 1);

        let convs_after = manager.list_conversations().unwrap();
        assert_eq!(convs_after[0].unread_count, 0);
    }
}
