"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.generateSdk = generateSdk;
const utils_1 = require("./utils");
function generateSdk(multiIr) {
    const lines = [
        `// GENERATED CODE – DO NOT EDIT.`,
        `import { useAxiomQuery, useAxiomMutation } from 'atmx-react';`,
        `import type { AxiomQueryDef } from 'atmx-react';`,
        `import * as models from './models';\n`
    ];
    // 1. Generate individual Module Classes for each namespace
    for (const [ns, ir] of Object.entries(multiIr)) {
        const pascalNs = (0, utils_1.pascalCase)(ns);
        lines.push(`export class ${pascalNs}Module {`);
        const endpoints = ir.endpoints || [];
        endpoints.forEach((ep) => {
            lines.push(generateEndpointHook(ep, ns, pascalNs));
        });
        lines.push(`}\n`);
    }
    // 2. Generate the central SDK Class
    lines.push(`export class AxiomSdk {`);
    for (const ns of Object.keys(multiIr)) {
        lines.push(`  public readonly ${(0, utils_1.camelCase)(ns)} = new ${(0, utils_1.pascalCase)(ns)}Module();`);
    }
    lines.push(`}\n`);
    lines.push(`export const sdk = new AxiomSdk();`);
    return lines.join('\n');
}
function generateEndpointHook(ep, ns, pascalNs) {
    const isQuery = ep.method ? ep.method.toUpperCase() === 'GET' : true;
    const hookName = `use${(0, utils_1.pascalCase)(ep.name)}${!isQuery ? 'Mutation' : ''}`;
    const rawReturnType = (0, utils_1.mapTypeToTs)(ep.returnType, pascalNs);
    const returnType = rawReturnType === 'void' || rawReturnType === 'any'
        ? rawReturnType
        : prefixModels(rawReturnType);
    const namespacedReturnType = (returnType === 'void' || returnType === 'any') ? returnType : returnType;
    const params = ep.parameters || [];
    const hasParams = params.length > 0;
    const argType = hasParams ? `{ ${params.map(p => `${(0, utils_1.camelCase)(p.name)}${p.isOptional ? '?' : ''}: ${prefixModels((0, utils_1.mapTypeToTs)(p.typeRef, pascalNs))}`).join(', ')} }` : 'void';
    const factoryLogic = generateQueryDefFactory(ep, namespacedReturnType, ns);
    if (isQuery) {
        return `
  ${hookName}(args: ${argType}, options?: { enabled?: boolean }) {
    const factory = ${factoryLogic};
    return useAxiomQuery<${namespacedReturnType}>(factory(args), options);
  }\n`;
    }
    else {
        return `
  ${hookName}() {
    const factory = ${factoryLogic};
    return useAxiomMutation<${namespacedReturnType}, ${argType}>(factory);
  }\n`;
    }
}
function generateQueryDefFactory(ep, returnType, ns) {
    let pathLogic = `let processedPath = "${ep.path || '/'}";`;
    let queryLogic = `const q = new URLSearchParams();`;
    const params = ep.parameters || [];
    params.forEach(p => {
        const key = (0, utils_1.camelCase)(p.name);
        if (p.source === 'path')
            pathLogic += `\n      processedPath = processedPath.replace("{${p.name}}", String((args as any).${key}));`;
        if (p.source === 'query')
            queryLogic += `\n      if ((args as any).${key} !== undefined) q.append("${p.name}", String((args as any).${key}));`;
    });
    const bodyParam = params.find(p => p.source === 'body');
    const payloadLogic = bodyParam ? `const payload = (args as any).${(0, utils_1.camelCase)(bodyParam.name)};` : `const payload = undefined;`;
    const decLogic = generateLambda(ep.returnType, 'fromJson', ns);
    const serLogic = bodyParam ? generateLambda(bodyParam.typeRef, 'toJson', ns) : `(p: any) => p`;
    // Notice we inject `namespace: "${ns}"` here for the React Hooks
    return `(args: any): AxiomQueryDef<${returnType}> => {
      ${pathLogic}
      ${queryLogic}
      if (q.toString()) processedPath += "?" + q.toString();
      ${payloadLogic}
      return {
        namespace: "${ns}",
        endpointId: ${ep.id},
        method: "${ep.method ? ep.method.toUpperCase() : 'GET'}",
        path: processedPath,
        payload: payload,
        args: args as any,
        decoder: ${decLogic},
        serializer: ${serLogic}
      };
    }`;
}
function generateLambda(typeRef, mode, ns) {
    if (!typeRef || !typeRef.kind || typeRef.kind === 'void')
        return mode === 'fromJson' ? `() => undefined` : `(p: any) => p`;
    if (typeRef.kind === 'list' && typeRef.value?.kind === 'named') {
        return `(data: any[]) => data.map(models.Mappers.${(0, utils_1.camelCase)(ns)}.${(0, utils_1.pascalCase)(typeRef.value.value)}.${mode})`;
    }
    if (typeRef.kind === 'named') {
        return `models.Mappers.${(0, utils_1.camelCase)(ns)}.${(0, utils_1.pascalCase)(typeRef.value)}.${mode}`;
    }
    return `(data: any) => data`;
}
function prefixModels(type) {
    if (!type || type === 'void' || type === 'any')
        return type;
    // handle arrays
    if (type.endsWith('[]')) {
        return `${prefixModels(type.slice(0, -2))}[]`;
    }
    // already prefixed
    if (type.startsWith('models.'))
        return type;
    return `models.${type}`;
}
