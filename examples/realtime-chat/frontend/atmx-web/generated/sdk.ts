// GENERATED CODE – DO NOT EDIT.
/* eslint-disable @typescript-eslint/no-explicit-any */
import * as models from './models';

export const realtimeChatModule = {
  axiom: {
    setAuthToken(methodName: string, token: string) {
      (window as any).atmx?.setAuthToken("realtime-chat", methodName, token);
    },
    clearAuthToken(methodName: string) {
      (window as any).atmx?.clearAuthToken("realtime-chat", methodName);
    },
    connect(methodName: string, args?: Record<string, any>) {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      (window as any).atmx?.connect(`realtime-chat.${methodName}(${argsStr})`);
    },
    disconnect(methodName: string, args?: Record<string, any>) {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      (window as any).atmx?.disconnect(`realtime-chat.${methodName}(${argsStr})`);
    },
    send(methodName: string, payload: any, args?: Record<string, any>) {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      (window as any).atmx?.send(`realtime-chat.${methodName}(${argsStr})`, payload);
    }
  },

  handleConnections(args?: Record<string, any>): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
    return `realtime-chat.handleConnections(${argsStr})`;
  },
};

export const sdk = {
  realtimeChat: realtimeChatModule,
};

export const AxiomDefaultConfig = {
  contracts: {
    "realtime-chat": {
      contractUrl: "/realtime-chat.axiom",
      baseUrl: "http://localhost:8000"
    },
  }
};
