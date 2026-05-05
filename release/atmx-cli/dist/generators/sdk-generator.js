"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.generateSdk = generateSdk;
const utils_1 = require("./utils");
function generateSdk(multiIr, isReact = false) {
    const lines = [
        `// GENERATED CODE – DO NOT EDIT.`,
        `/* eslint-disable @typescript-eslint/no-explicit-any */`,
        `/* eslint-disable @typescript-eslint/no-unused-vars */`,
        `import * as models from './models';\n`,
    ];
    if (isReact) {
        // ✨ FIX: Auto-import the auth helpers and QueryManager directly
        lines.push(`import { useAxiomQuery, useAxiomMutation, setAuthToken, clearAuthToken, axiomQueryManager } from 'atmx-react';`);
        lines.push(`import type { AxiomQueryDef } from 'atmx-react';\n`);
    }
    for (const [ns, ir] of Object.entries(multiIr)) {
        const camelNs = (0, utils_1.camelCase)(ns);
        lines.push(`export const ${camelNs}Module = {`);
        lines.push(`  axiom: {`);
        if (isReact) {
            lines.push(`    setAuthToken(methodName: string, token: string) {`);
            lines.push(`      setAuthToken("${ns}", methodName, token);`);
            lines.push(`    },`);
            lines.push(`    clearAuthToken(methodName: string) {`);
            lines.push(`      clearAuthToken("${ns}", methodName);`);
            lines.push(`    },`);
            lines.push(`    connect(methodName: string, args?: Record<string, any>) {`);
            // ✨ FIX: Use the specific module name instead of 'this'
            lines.push(`      const def = (${camelNs}Module as any)[\`get\${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def\`](args);`);
            lines.push(`      axiomQueryManager.connect(def);`);
            lines.push(`    },`);
            lines.push(`    disconnect(methodName: string, args?: Record<string, any>) {`);
            lines.push(`      const def = (${camelNs}Module as any)[\`get\${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def\`](args);`);
            lines.push(`      axiomQueryManager.disconnect(def);`);
            lines.push(`    },`);
            lines.push(`    send(methodName: string, payload: any, args?: Record<string, any>) {`);
            lines.push(`      const def = (${camelNs}Module as any)[\`get\${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def\`](args);`);
            lines.push(`      axiomQueryManager.send(def, payload);`);
            lines.push(`    }`);
        }
        else {
            lines.push(`    setAuthToken(methodName: string, token: string) {`);
            lines.push(`      (window as any).atmx?.setAuthToken("${ns}", methodName, token);`);
            lines.push(`    },`);
            lines.push(`    clearAuthToken(methodName: string) {`);
            lines.push(`      (window as any).atmx?.clearAuthToken("${ns}", methodName);`);
            lines.push(`    },`);
            lines.push(`    connect(methodName: string, args?: Record<string, any>) {`);
            lines.push(`      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';`);
            lines.push(`      (window as any).atmx?.connect(\`${ns}.\${methodName}(\${argsStr})\`);`);
            lines.push(`    },`);
            lines.push(`    disconnect(methodName: string, args?: Record<string, any>) {`);
            lines.push(`      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';`);
            lines.push(`      (window as any).atmx?.disconnect(\`${ns}.\${methodName}(\${argsStr})\`);`);
            lines.push(`    },`);
            lines.push(`    send(methodName: string, payload: any, args?: Record<string, any>) {`);
            lines.push(`      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';`);
            lines.push(`      (window as any).atmx?.send(\`${ns}.\${methodName}(\${argsStr})\`, payload);`);
            lines.push(`    }`);
        }
        lines.push(`  },`);
        const endpointsMap = ir.endpoints || {};
        const endpoints = Array.isArray(endpointsMap)
            ? endpointsMap
            : Object.values(endpointsMap);
        endpoints.forEach((ep) => {
            lines.push(generateEndpointMethod(ep, ns, camelNs, isReact));
        });
        lines.push(`};\n`);
    }
    lines.push(`export const sdk = {`);
    for (const ns of Object.keys(multiIr)) {
        lines.push(`  ${(0, utils_1.camelCase)(ns)}: ${(0, utils_1.camelCase)(ns)}Module,`);
    }
    lines.push(`};\n`);
    lines.push(`export const AxiomDefaultConfig = {`);
    lines.push(`  contracts: {`);
    for (const ns of Object.keys(multiIr)) {
        lines.push(`    "${ns}": {`);
        lines.push(`      contractUrl: "/${ns}.axiom",`);
        lines.push(`      baseUrl: "http://localhost:8000"`);
        lines.push(`    },`);
    }
    lines.push(`  }`);
    lines.push(`};\n`);
    return lines.join("\n");
}
function generateEndpointMethod(ep, ns, camelNs, isReact) {
    const rawParams = ep.parameters || [];
    const params = Array.isArray(rawParams)
        ? rawParams
        : Object.values(rawParams);
    // ✨ FIX: Treat "WS" as a query (subscription) instead of a mutation!
    const isQuery = ep.method
        ? ["GET", "WS"].includes(ep.method.toUpperCase())
        : true;
    const rawReturnType = (0, utils_1.mapTypeToTs)(ep.returnType, camelNs);
    const returnType = rawReturnType === "void" || rawReturnType === "any"
        ? rawReturnType
        : prefixModels(rawReturnType);
    const argsMapping = params
        .map((p) => `if (args && '${(0, utils_1.camelCase)(p.name)}' in args) { mappedArgs["${p.name}"] = (args as any)["${(0, utils_1.camelCase)(p.name)}"]; delete mappedArgs["${(0, utils_1.camelCase)(p.name)}"]; }`)
        .join("\n    ");
    if (params.length === 0) {
        if (isReact) {
            const decLogic = generateLambda(ep.returnType, "fromJson", camelNs);
            return `
  get${(0, utils_1.pascalCase)(ep.name)}Def(args?: Record<string, any>): AxiomQueryDef<${returnType}> {
    return {
      namespace: "${ns}", name: "${ep.name}", endpointId: ${ep.id},
      method: "${ep.method ? ep.method.toUpperCase() : "GET"}", path: "${ep.path}",
      args: args || {}, decoder: ${decLogic}, serializer: (p: any) => p, isStream: ${ep.isStream === true}
    };
  },
  use${(0, utils_1.pascalCase)(ep.name)}(${isQuery ? "options?: { enabled?: boolean }" : ""}) {
    ${isQuery ? `return useAxiomQuery<${returnType}>(this.get${(0, utils_1.pascalCase)(ep.name)}Def(), options);` : `return useAxiomMutation<${returnType}, void | Record<string,any>>((a) => this.get${(0, utils_1.pascalCase)(ep.name)}Def(a));`}
  },`;
        }
        else {
            return `
  ${(0, utils_1.camelCase)(ep.name)}(args?: Record<string, any>): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
    return \`${ns}.${ep.name}(\${argsStr})\`;
  },`;
        }
    }
    const argType = `{ ${params.map((p) => `${(0, utils_1.camelCase)(p.name)}${p.isOptional ? "?" : ""}: ${prefixModels((0, utils_1.mapTypeToTs)(p.typeRef, camelNs))}`).join(", ")} }`;
    if (isReact) {
        const bodyParam = params.find((p) => p.source === "body");
        const payloadLogic = bodyParam
            ? `const payload = (args as any)?.${(0, utils_1.camelCase)(bodyParam.name)};`
            : `const payload = undefined;`;
        const decLogic = generateLambda(ep.returnType, "fromJson", camelNs);
        const serLogic = bodyParam
            ? generateLambda(bodyParam.typeRef, "toJson", camelNs)
            : `(p: any) => p`;
        return `
  get${(0, utils_1.pascalCase)(ep.name)}Def(args?: ${argType}): AxiomQueryDef<${returnType}> {
    ${payloadLogic}
    const mappedArgs: any = { ...(args || {}) };
    ${argsMapping}

    return {
      namespace: "${ns}", name: "${ep.name}", endpointId: ${ep.id},
      method: "${ep.method ? ep.method.toUpperCase() : "GET"}", path: "${ep.path}",
      payload: payload, args: mappedArgs, decoder: ${decLogic}, serializer: ${serLogic}, isStream: ${ep.isStream === true}
    };
  },
  use${(0, utils_1.pascalCase)(ep.name)}(${isQuery ? `args?: ${argType}, options?: { enabled?: boolean }` : `args?: ${argType}`}) {
    ${isQuery ? `return useAxiomQuery<${returnType}>(this.get${(0, utils_1.pascalCase)(ep.name)}Def(args), options);` : `return useAxiomMutation<${returnType}, ${argType}>((a) => this.get${(0, utils_1.pascalCase)(ep.name)}Def(a || args));`}
  },`;
    }
    else {
        return `
  ${(0, utils_1.camelCase)(ep.name)}(args?: ${argType}): string {
    const mappedArgs: any = { ...(args || {}) };
    ${argsMapping}
    const argsStr = Object.keys(mappedArgs).length > 0 ? JSON.stringify(mappedArgs) : '';
    return \`${ns}.${ep.name}(\${argsStr})\`;
  },`;
    }
}
function generateLambda(typeRef, mode, ns) {
    if (!typeRef || !typeRef.kind || typeRef.kind === "void")
        return mode === "fromJson" ? `() => undefined` : `(p: any) => p`;
    if (typeRef.kind === "list" && typeRef.value?.kind === "named")
        return `(data: any[]) => data.map(models.Mappers.${ns}.${(0, utils_1.pascalCase)(typeRef.value.value)}.${mode})`;
    if (typeRef.kind === "named")
        return `models.Mappers.${ns}.${(0, utils_1.pascalCase)(typeRef.value)}.${mode}`;
    return `(data: any) => data`;
}
function prefixModels(type) {
    const primitives = [
        "string",
        "number",
        "boolean",
        "Date",
        "Uint8Array",
        "void",
        "any",
    ];
    if (!type || primitives.includes(type))
        return type;
    if (type.endsWith("[]"))
        return `${prefixModels(type.slice(0, -2))}[]`;
    if (type.startsWith("models."))
        return type;
    return `models.${type}`;
}
