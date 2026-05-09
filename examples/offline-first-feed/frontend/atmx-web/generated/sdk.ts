// GENERATED CODE – DO NOT EDIT.
/* eslint-disable @typescript-eslint/no-explicit-any */
/* eslint-disable @typescript-eslint/no-unused-vars */
import * as models from './models';

export const feedModule = {
  axiom: {
    setAuthToken(methodName: string, token: string) {
      (window as any).atmx?.setAuthToken("feed", methodName, token);
    },
    clearAuthToken(methodName: string) {
      (window as any).atmx?.clearAuthToken("feed", methodName);
    },
    connect(methodName: string, args?: Record<string, any>) {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      (window as any).atmx?.connect(`feed.${methodName}(${argsStr})`);
    },
    disconnect(methodName: string, args?: Record<string, any>) {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      (window as any).atmx?.disconnect(`feed.${methodName}(${argsStr})`);
    },
    send(methodName: string, payload: any, args?: Record<string, any>) {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      (window as any).atmx?.send(`feed.${methodName}(${argsStr})`, payload);
    }
  },

  getPosts(args?: Record<string, any>): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
    return `feed.get_posts(${argsStr})`;
  },
};

export const sdk = {
  feed: feedModule,
};

export const AxiomDefaultConfig = {
  contracts: {
    "feed": {
      contractUrl: "/feed.axiom",
      baseUrl: "http://localhost:8000"
    },
  }
};
