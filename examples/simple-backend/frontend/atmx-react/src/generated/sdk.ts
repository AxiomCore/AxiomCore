// GENERATED CODE – DO NOT EDIT.
/* eslint-disable @typescript-eslint/no-explicit-any */
import * as models from './models';

import { useAxiomQuery, useAxiomMutation, setAuthToken, clearAuthToken } from 'atmx-react';
import type { AxiomQueryDef } from 'atmx-react';

export const pyExampleModule = {
  setAuthToken(methodName: string, token: string) {
    setAuthToken("py-example", methodName, token);
  },
  clearAuthToken(methodName: string) {
    clearAuthToken("py-example", methodName);
  },

  getCreateItemDef(args?: { item: models.pyExample.ItemCreate }): AxiomQueryDef<models.pyExample.Item> {
    const payload = (args as any)?.item;
    const mappedArgs: any = { ...(args || {}) };
    if (args && 'item' in args) { mappedArgs["item"] = (args as any)["item"]; delete mappedArgs["item"]; }

    return {
      namespace: "py-example", name: "create_item", endpointId: 2,
      method: "POST", path: "/items",
      payload: payload, args: mappedArgs, decoder: models.Mappers.pyExample.Item.fromJson, serializer: models.Mappers.pyExample.ItemCreate.toJson, isStream: false
    };
  },
  useCreateItemMutation(args?: { item: models.pyExample.ItemCreate }, options?: { enabled?: boolean }) {
    return useAxiomMutation<models.pyExample.Item, { item: models.pyExample.ItemCreate }>((a) => this.getCreateItemDef(a || args));
  },

  getDeleteItemDef(args?: { itemId: string }): AxiomQueryDef<any> {
    const payload = undefined;
    const mappedArgs: any = { ...(args || {}) };
    if (args && 'itemId' in args) { mappedArgs["item_id"] = (args as any)["itemId"]; delete mappedArgs["itemId"]; }

    return {
      namespace: "py-example", name: "delete_item", endpointId: 5,
      method: "DELETE", path: "/items/{item_id}",
      payload: payload, args: mappedArgs, decoder: (data: any) => data, serializer: (p: any) => p, isStream: false
    };
  },
  useDeleteItemMutation(args?: { itemId: string }, options?: { enabled?: boolean }) {
    return useAxiomMutation<any, { itemId: string }>((a) => this.getDeleteItemDef(a || args));
  },

  getGetItemDef(args?: { itemId: string }): AxiomQueryDef<models.pyExample.Item> {
    const payload = undefined;
    const mappedArgs: any = { ...(args || {}) };
    if (args && 'itemId' in args) { mappedArgs["item_id"] = (args as any)["itemId"]; delete mappedArgs["itemId"]; }

    return {
      namespace: "py-example", name: "get_item", endpointId: 4,
      method: "GET", path: "/items/{item_id}",
      payload: payload, args: mappedArgs, decoder: models.Mappers.pyExample.Item.fromJson, serializer: (p: any) => p, isStream: false
    };
  },
  useGetItem(args?: { itemId: string }, options?: { enabled?: boolean }) {
    return useAxiomQuery<models.pyExample.Item>(this.getGetItemDef(args), options);
  },

  getListItemsDef(args?: { skip?: number, limit?: number, search?: string }): AxiomQueryDef<models.pyExample.Item[]> {
    const payload = undefined;
    const mappedArgs: any = { ...(args || {}) };
    if (args && 'skip' in args) { mappedArgs["skip"] = (args as any)["skip"]; delete mappedArgs["skip"]; }
    if (args && 'limit' in args) { mappedArgs["limit"] = (args as any)["limit"]; delete mappedArgs["limit"]; }
    if (args && 'search' in args) { mappedArgs["search"] = (args as any)["search"]; delete mappedArgs["search"]; }

    return {
      namespace: "py-example", name: "list_items", endpointId: 3,
      method: "GET", path: "/items",
      payload: payload, args: mappedArgs, decoder: (data: any[]) => data.map(models.Mappers.pyExample.Item.fromJson), serializer: (p: any) => p, isStream: false
    };
  },
  useListItems(args?: { skip?: number, limit?: number, search?: string }, options?: { enabled?: boolean }) {
    return useAxiomQuery<models.pyExample.Item[]>(this.getListItemsDef(args), options);
  },

  getListUsersDef(args?: Record<string, any>): AxiomQueryDef<any> {
    return {
      namespace: "py-example", name: "list_users", endpointId: 7,
      method: "GET", path: "/admin/users",
      args: args || {}, decoder: (data: any) => data, serializer: (p: any) => p, isStream: false
    };
  },
  useListUsers(options?: { enabled?: boolean }) {
    return useAxiomQuery<any>(this.getListUsersDef(), options);
  },

  getLoginDef(args?: Record<string, any>): AxiomQueryDef<models.pyExample.Token> {
    return {
      namespace: "py-example", name: "login", endpointId: 1,
      method: "POST", path: "/login",
      args: args || {}, decoder: models.Mappers.pyExample.Token.fromJson, serializer: (p: any) => p, isStream: false
    };
  },
  useLoginMutation(options?: { enabled?: boolean }) {
    return useAxiomMutation<models.pyExample.Token, void | Record<string,any>>((a) => this.getLoginDef(a));
  },

  getRegisterDef(args?: { user: models.pyExample.UserCreate }): AxiomQueryDef<models.pyExample.User> {
    const payload = (args as any)?.user;
    const mappedArgs: any = { ...(args || {}) };
    if (args && 'user' in args) { mappedArgs["user"] = (args as any)["user"]; delete mappedArgs["user"]; }

    return {
      namespace: "py-example", name: "register", endpointId: 0,
      method: "POST", path: "/register",
      payload: payload, args: mappedArgs, decoder: models.Mappers.pyExample.User.fromJson, serializer: models.Mappers.pyExample.UserCreate.toJson, isStream: false
    };
  },
  useRegisterMutation(args?: { user: models.pyExample.UserCreate }, options?: { enabled?: boolean }) {
    return useAxiomMutation<models.pyExample.User, { user: models.pyExample.UserCreate }>((a) => this.getRegisterDef(a || args));
  },

  getSendEmailDef(args?: { backgroundTasks: any }): AxiomQueryDef<any> {
    const payload = undefined;
    const mappedArgs: any = { ...(args || {}) };
    if (args && 'backgroundTasks' in args) { mappedArgs["background_tasks"] = (args as any)["backgroundTasks"]; delete mappedArgs["backgroundTasks"]; }

    return {
      namespace: "py-example", name: "send_email", endpointId: 6,
      method: "POST", path: "/send-email",
      payload: payload, args: mappedArgs, decoder: (data: any) => data, serializer: (p: any) => p, isStream: false
    };
  },
  useSendEmailMutation(args?: { backgroundTasks: any }, options?: { enabled?: boolean }) {
    return useAxiomMutation<any, { backgroundTasks: any }>((a) => this.getSendEmailDef(a || args));
  },

  getWebsocketEndpointDef(args?: Record<string, any>): AxiomQueryDef<void> {
    return {
      namespace: "py-example", name: "websocket_endpoint", endpointId: 8,
      method: "WS", path: "/ws",
      args: args || {}, decoder: () => undefined, serializer: (p: any) => p, isStream: false
    };
  },
  useWebsocketEndpointMutation(options?: { enabled?: boolean }) {
    return useAxiomMutation<void, void | Record<string,any>>((a) => this.getWebsocketEndpointDef(a));
  },
};

export const sdk = {
  pyExample: pyExampleModule,
};

export const AxiomDefaultConfig = {
  contracts: {
    "py-example": {
      contractUrl: "/py-example.axiom",
      baseUrl: "http://localhost:8000"
    },
  }
};
