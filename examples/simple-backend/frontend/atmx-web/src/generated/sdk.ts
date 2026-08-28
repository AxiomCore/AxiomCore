// GENERATED CODE – DO NOT EDIT.
/* eslint-disable @typescript-eslint/no-explicit-any */
/* eslint-disable @typescript-eslint/no-unused-vars */

import * as models from './models.js';

export const firstProjectModule = {
  axiom: {
    setAuthToken(methodName: string, token: string) {
      (window as any).atmx?.setAuthToken("first-project", methodName, token);
    },
    clearAuthToken(methodName: string) {
      (window as any).atmx?.clearAuthToken("first-project", methodName);
    },
    connect(methodName: string, args?: Record<string, any>) {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      (window as any).atmx?.connect(`first-project.${methodName}(${argsStr})`);
    },
    disconnect(methodName: string, args?: Record<string, any>) {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      (window as any).atmx?.disconnect(`first-project.${methodName}(${argsStr})`);
    },
    send(methodName: string, payload: any, args?: Record<string, any>) {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      (window as any).atmx?.send(`first-project.${methodName}(${argsStr})`, payload);
    }
  },

  createItem: Object.assign(
    (args?: Record<string, any>): string => {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      return `first-project.create_item(${argsStr})`;
    },
    {
      invalidate(args?: Record<string, any>) {
        (window as any).atmx?.invalidate("first-project.create_item", args);
      },
      setData(data: any, args?: Record<string, any>) {
        (window as any).atmx?.setQueryData("first-project.create_item", args || {}, data);
      },
      mutate(payload: any = {}, args?: Record<string, any>): Promise<any> {
        return (window as any).atmx?.mutate("first-project.create_item", args, payload);
      }
    }
  ),
  deleteItem: Object.assign(
    (args?: Record<string, any>): string => {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      return `first-project.delete_item(${argsStr})`;
    },
    {
      invalidate(args?: Record<string, any>) {
        (window as any).atmx?.invalidate("first-project.delete_item", args);
      },
      setData(data: any, args?: Record<string, any>) {
        (window as any).atmx?.setQueryData("first-project.delete_item", args || {}, data);
      },
      mutate(payload: any = {}, args?: Record<string, any>): Promise<any> {
        return (window as any).atmx?.mutate("first-project.delete_item", args, payload);
      }
    }
  ),
  getItem: Object.assign(
    (args?: Record<string, any>): string => {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      return `first-project.get_item(${argsStr})`;
    },
    {
      invalidate(args?: Record<string, any>) {
        (window as any).atmx?.invalidate("first-project.get_item", args);
      },
      setData(data: any, args?: Record<string, any>) {
        (window as any).atmx?.setQueryData("first-project.get_item", args || {}, data);
      },
      mutate(payload: any = {}, args?: Record<string, any>): Promise<any> {
        return (window as any).atmx?.mutate("first-project.get_item", args, payload);
      }
    }
  ),
  listItems: Object.assign(
    (args?: Record<string, any>): string => {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      return `first-project.list_items(${argsStr})`;
    },
    {
      invalidate(args?: Record<string, any>) {
        (window as any).atmx?.invalidate("first-project.list_items", args);
      },
      setData(data: any, args?: Record<string, any>) {
        (window as any).atmx?.setQueryData("first-project.list_items", args || {}, data);
      },
      mutate(payload: any = {}, args?: Record<string, any>): Promise<any> {
        return (window as any).atmx?.mutate("first-project.list_items", args, payload);
      }
    }
  ),
  listUsers: Object.assign(
    (args?: Record<string, any>): string => {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      return `first-project.list_users(${argsStr})`;
    },
    {
      invalidate(args?: Record<string, any>) {
        (window as any).atmx?.invalidate("first-project.list_users", args);
      },
      setData(data: any, args?: Record<string, any>) {
        (window as any).atmx?.setQueryData("first-project.list_users", args || {}, data);
      },
      mutate(payload: any = {}, args?: Record<string, any>): Promise<any> {
        return (window as any).atmx?.mutate("first-project.list_users", args, payload);
      }
    }
  ),
  login: Object.assign(
    (args?: Record<string, any>): string => {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      return `first-project.login(${argsStr})`;
    },
    {
      invalidate(args?: Record<string, any>) {
        (window as any).atmx?.invalidate("first-project.login", args);
      },
      setData(data: any, args?: Record<string, any>) {
        (window as any).atmx?.setQueryData("first-project.login", args || {}, data);
      },
      mutate(payload: any = {}, args?: Record<string, any>): Promise<any> {
        return (window as any).atmx?.mutate("first-project.login", args, payload);
      }
    }
  ),
  register: Object.assign(
    (args?: Record<string, any>): string => {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      return `first-project.register(${argsStr})`;
    },
    {
      invalidate(args?: Record<string, any>) {
        (window as any).atmx?.invalidate("first-project.register", args);
      },
      setData(data: any, args?: Record<string, any>) {
        (window as any).atmx?.setQueryData("first-project.register", args || {}, data);
      },
      mutate(payload: any = {}, args?: Record<string, any>): Promise<any> {
        return (window as any).atmx?.mutate("first-project.register", args, payload);
      }
    }
  ),
  sendEmail: Object.assign(
    (args?: Record<string, any>): string => {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      return `first-project.send_email(${argsStr})`;
    },
    {
      invalidate(args?: Record<string, any>) {
        (window as any).atmx?.invalidate("first-project.send_email", args);
      },
      setData(data: any, args?: Record<string, any>) {
        (window as any).atmx?.setQueryData("first-project.send_email", args || {}, data);
      },
      mutate(payload: any = {}, args?: Record<string, any>): Promise<any> {
        return (window as any).atmx?.mutate("first-project.send_email", args, payload);
      }
    }
  ),
  websocketEndpoint: Object.assign(
    (args?: Record<string, any>): string => {
      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
      return `first-project.websocket_endpoint(${argsStr})`;
    },
    {
      invalidate(args?: Record<string, any>) {
        (window as any).atmx?.invalidate("first-project.websocket_endpoint", args);
      },
      setData(data: any, args?: Record<string, any>) {
        (window as any).atmx?.setQueryData("first-project.websocket_endpoint", args || {}, data);
      },
      mutate(payload: any = {}, args?: Record<string, any>): Promise<any> {
        return (window as any).atmx?.mutate("first-project.websocket_endpoint", args, payload);
      }
    }
  ),
};

const internalSdk: Record<string, any> = {
  "first-project": firstProjectModule,
  firstProject: firstProjectModule,
};

// ✨ The Magic Proxy: Safely intercepts Alpine.js evaluations during boot!
export const sdk = new Proxy(internalSdk, {
  get(target: any, prop: string, receiver: any) {
    if (prop in target) {
      return Reflect.get(target, prop, receiver);
    }
    // Create a dynamic namespace proxy
    return new Proxy({}, {
      get(subTarget: any, subProp: string) {
        // Return a callable function that returns the string definition
        const routeFn = (args?: Record<string, any>) => {
          const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
          return `${String(prop)}.${String(subProp)}(${argsStr})`;
        };
        // Attach typed helper methods directly to the function!
        routeFn.invalidate = (args?: Record<string, any>) => {
          (window as any).atmx?.invalidate(`${String(prop)}.${String(subProp)}`, args);
        };
        routeFn.setData = (data: any, args?: Record<string, any>) => {
          (window as any).atmx?.setQueryData(`${String(prop)}.${String(subProp)}`, args || {}, data);
        };
        routeFn.mutate = (payload: any = {}, args?: Record<string, any>): Promise<any> => {
          return (window as any).atmx?.mutate(`${String(prop)}.${String(subProp)}`, args, payload);
        };
        return routeFn;
      }
    });
  }
});

// Auto-attach to window for Alpine.js immediate hydration
if (typeof window !== "undefined") {
  (window as any).sdk = sdk;
}

export const AxiomDefaultConfig = {
  contracts: {
    "first-project": {
      contractUrl: "/first-project.axiom",
      baseUrl: "http://localhost:8080",
      contractSignature: "jIb10vTMi1xEPxFPmb3iEta+Ykyi6t3lRD5AQ1ylBdeddt0nZ87MNIgQ6BlWWDsaNQQg0OsJOeuFvYKwAWjYAA==",
      contractPublicKey: "w7iC5GFG6O2kEUeE7U/wnH6IseQj2zLR91J4fYzci9w="
    },
  }
};
