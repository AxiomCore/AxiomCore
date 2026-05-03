export class PyExampleModule {
    /** RPC String Generator for <AxQuery> or <AxMutate> */
    createItem(args) {
        const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
        return `pyExample.create_item(${argsStr})`;
    }
    /** RPC String Generator for <AxQuery> or <AxMutate> */
    deleteItem(args) {
        const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
        return `pyExample.delete_item(${argsStr})`;
    }
    /** RPC String Generator for <AxQuery> or <AxMutate> */
    getItem(args) {
        const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
        return `pyExample.get_item(${argsStr})`;
    }
    /** RPC String Generator for <AxQuery> or <AxMutate> */
    listItems(args) {
        const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
        return `pyExample.list_items(${argsStr})`;
    }
    /** RPC String Generator for <AxQuery> or <AxMutate> */
    listUsers() {
        return `pyExample.list_users()`;
    }
    /** RPC String Generator for <AxQuery> or <AxMutate> */
    login() {
        return `pyExample.login()`;
    }
    /** RPC String Generator for <AxQuery> or <AxMutate> */
    register(args) {
        const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
        return `pyExample.register(${argsStr})`;
    }
    /** RPC String Generator for <AxQuery> or <AxMutate> */
    sendEmail(args) {
        const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
        return `pyExample.send_email(${argsStr})`;
    }
    /** RPC String Generator for <AxQuery> or <AxMutate> */
    websocketEndpoint() {
        return `pyExample.websocket_endpoint()`;
    }
}
export class AxiomSdk {
    pyExample = new PyExampleModule();
}
export const sdk = new AxiomSdk();
