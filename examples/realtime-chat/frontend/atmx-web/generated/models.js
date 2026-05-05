// GENERATED CODE – DO NOT EDIT.
/* eslint-disable @typescript-eslint/no-explicit-any */
export const Mappers = {
    realtimeChat: {
        ChatMessage: {
            fromJson: (json) => ({
                content: json["content"],
                sender: json["sender"],
                timestamp: json["timestamp"],
            }),
            toJson: (obj) => ({
                "content": obj.content,
                "sender": obj.sender,
                "timestamp": obj.timestamp,
            })
        },
    },
};
