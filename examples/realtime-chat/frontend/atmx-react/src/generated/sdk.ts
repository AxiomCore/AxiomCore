// GENERATED CODE – DO NOT EDIT.
/* eslint-disable @typescript-eslint/no-explicit-any */
/* eslint-disable @typescript-eslint/no-unused-vars */
import * as models from "./models";

import {
  useAxiomQuery,
  useAxiomMutation,
  setAuthToken,
  clearAuthToken,
  axiomQueryManager,
} from "atmx-react";
import type { AxiomQueryDef } from "atmx-react";

export const realtimeChatModule = {
  axiom: {
    setAuthToken(methodName: string, token: string) {
      setAuthToken("realtime-chat", methodName, token);
    },
    clearAuthToken(methodName: string) {
      clearAuthToken("realtime-chat", methodName);
    },
    connect(methodName: string, args?: Record<string, any>) {
      const def = (realtimeChatModule as any)[
        `get${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def`
      ](args);
      axiomQueryManager.connect(def);
    },
    disconnect(methodName: string, args?: Record<string, any>) {
      const def = (realtimeChatModule as any)[
        `get${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def`
      ](args);
      axiomQueryManager.disconnect(def);
    },
    send(methodName: string, payload: any, args?: Record<string, any>) {
      const def = (realtimeChatModule as any)[
        `get${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def`
      ](args);
      axiomQueryManager.send(def, payload);
    },
  },

  getHandleConnectionsDef(
    args?: Record<string, any>,
  ): AxiomQueryDef<models.realtimeChat.ChatMessage> {
    return {
      namespace: "realtime-chat",
      name: "handleConnections",
      endpointId: 0,
      method: "WS",
      path: "/ws",
      args: args || {},
      decoder: models.Mappers.realtimeChat.ChatMessage.fromJson,
      serializer: (p: any) => p,
      isStream: false,
    };
  },
  useHandleConnections(options?: { enabled?: boolean }) {
    return useAxiomQuery<models.realtimeChat.ChatMessage>(
      this.getHandleConnectionsDef(),
      options,
    );
  },
};

export const sdk = {
  realtimeChat: realtimeChatModule,
};

export const AxiomDefaultConfig = {
  contracts: {
    "realtime-chat": {
      contractUrl: "/realtime-chat.axiom",
      baseUrl: "http://localhost:8080",
    },
  },
};
