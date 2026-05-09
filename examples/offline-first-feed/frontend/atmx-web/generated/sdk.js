export const feedModule = {
    axiom: {
        setAuthToken(methodName, token) {
            window.atmx?.setAuthToken("feed", methodName, token);
        },
        clearAuthToken(methodName) {
            window.atmx?.clearAuthToken("feed", methodName);
        },
        connect(methodName, args) {
            const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
            window.atmx?.connect(`feed.${methodName}(${argsStr})`);
        },
        disconnect(methodName, args) {
            const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
            window.atmx?.disconnect(`feed.${methodName}(${argsStr})`);
        },
        send(methodName, payload, args) {
            const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
            window.atmx?.send(`feed.${methodName}(${argsStr})`, payload);
        }
    },
    getPosts(args) {
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
