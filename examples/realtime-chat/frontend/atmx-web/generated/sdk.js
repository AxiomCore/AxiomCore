export const realtimeChatModule = {
    axiom: {
        setAuthToken(methodName, token) {
            window.atmx?.setAuthToken("realtime-chat", methodName, token);
        },
        clearAuthToken(methodName) {
            window.atmx?.clearAuthToken("realtime-chat", methodName);
        },
        connect(methodName, args) {
            const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
            window.atmx?.connect(`realtime-chat.${methodName}(${argsStr})`);
        },
        disconnect(methodName, args) {
            const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
            window.atmx?.disconnect(`realtime-chat.${methodName}(${argsStr})`);
        },
        send(methodName, payload, args) {
            const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
            window.atmx?.send(`realtime-chat.${methodName}(${argsStr})`, payload);
        }
    },
    handleConnections(args) {
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
