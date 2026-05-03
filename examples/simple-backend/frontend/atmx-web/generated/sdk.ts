// GENERATED CODE – DO NOT EDIT.
import * as models from './models';

export class PyExampleModule {

  /** RPC String Generator for <AxQuery> or <AxMutate> */
  createItem(args?: { item?: models.PyExampleItemCreate }): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
    return `pyExample.create_item(${argsStr})`;
  }


  /** RPC String Generator for <AxQuery> or <AxMutate> */
  deleteItem(args?: { itemId?: any }): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
    return `pyExample.delete_item(${argsStr})`;
  }


  /** RPC String Generator for <AxQuery> or <AxMutate> */
  getItem(args?: { itemId?: any }): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
    return `pyExample.get_item(${argsStr})`;
  }


  /** RPC String Generator for <AxQuery> or <AxMutate> */
  listItems(args?: { skip?: any, limit?: any, search?: any }): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
    return `pyExample.list_items(${argsStr})`;
  }


  /** RPC String Generator for <AxQuery> or <AxMutate> */
  listUsers(): string {
    return `pyExample.list_users()`;
  }


  /** RPC String Generator for <AxQuery> or <AxMutate> */
  login(): string {
    return `pyExample.login()`;
  }


  /** RPC String Generator for <AxQuery> or <AxMutate> */
  register(args?: { user?: models.PyExampleUserCreate }): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
    return `pyExample.register(${argsStr})`;
  }


  /** RPC String Generator for <AxQuery> or <AxMutate> */
  sendEmail(args?: { backgroundTasks?: any }): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
    return `pyExample.send_email(${argsStr})`;
  }


  /** RPC String Generator for <AxQuery> or <AxMutate> */
  websocketEndpoint(): string {
    return `pyExample.websocket_endpoint()`;
  }

}

export class AxiomSdk {
  public readonly pyExample = new PyExampleModule();
}
export const sdk = new AxiomSdk();