// GENERATED CODE – DO NOT EDIT.
/* eslint-disable @typescript-eslint/no-explicit-any */

/* eslint-disable @typescript-eslint/no-namespace */

export namespace realtimeChat {

  export interface ChatMessage {
    content: string;
    sender: string;
    timestamp: string;
  }
  
}

export const Mappers: Record<string, any> = {
  realtimeChat: {
    ChatMessage: {
      fromJson: (json: any): realtimeChat.ChatMessage => ({
        content: json["content"],
        sender: json["sender"],
        timestamp: json["timestamp"],
      }),
      toJson: (obj: any): any => ({
        "content": obj.content,
        "sender": obj.sender,
        "timestamp": obj.timestamp,
      })
    },
  },
};
