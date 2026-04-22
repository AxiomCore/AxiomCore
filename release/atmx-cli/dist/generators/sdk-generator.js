"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.generateSdk = generateSdk;
const utils_1 = require("./utils");
function generateSdk(multiIr) {
    const lines = [
        `// GENERATED CODE – DO NOT EDIT.`,
        `import { useAxiomQuery, useAxiomMutation } from 'atmx-react';`,
        `import type { AxiomQueryDef } from 'atmx-react';`,
        `import * as models from './models';\n`,
    ];
    for (const [ns, ir] of Object.entries(multiIr)) {
        const pascalNs = (0, utils_1.pascalCase)(ns);
        lines.push(`export class ${pascalNs}Module {`);
        const endpoints = ir.endpoints || [];
        endpoints.forEach((ep) => {
            lines.push(generateEndpointHook(ep, ns, pascalNs));
        });
        lines.push(`}\n`);
    }
    lines.push(`export class AxiomSdk {`);
    for (const ns of Object.keys(multiIr)) {
        lines.push(`  public readonly ${(0, utils_1.camelCase)(ns)} = new ${(0, utils_1.pascalCase)(ns)}Module();`);
    }
    lines.push(`}\nexport const sdk = new AxiomSdk();`);
    return lines.join("\n");
}
function generateEndpointHook(ep, ns, pascalNs) {
    const isQuery = ep.method ? ep.method.toUpperCase() === "GET" : true;
    const rawReturnType = (0, utils_1.mapTypeToTs)(ep.returnType, pascalNs);
    const returnType = rawReturnType === "void" || rawReturnType === "any"
        ? rawReturnType
        : prefixModels(rawReturnType);
    const params = ep.parameters || [];
    const argType = params.length > 0
        ? `{ ${params.map((p) => `${(0, utils_1.camelCase)(p.name)}${p.isOptional ? "?" : ""}: ${prefixModels((0, utils_1.mapTypeToTs)(p.typeRef, pascalNs))}`).join(", ")} }`
        : "void";
    const factoryLogic = generateQueryDefFactory(ep, returnType, ns);
    // NEW: Every endpoint now generates BOTH a raw definition getter and a hook!
    return `
  /** Raw definition for <AxQuery> or <AxMutate> */
  get${(0, utils_1.pascalCase)(ep.name)}Def(args: ${argType}): AxiomQueryDef<${returnType}> {
    return (${factoryLogic})(args);
  }

  /** Reactive hook for functional components */
  use${(0, utils_1.pascalCase)(ep.name)}${!isQuery ? "Mutation" : ""}(${isQuery ? `args: ${argType}, options?: { enabled?: boolean }` : ""}) {
    const factory = ${factoryLogic};
    return ${isQuery ? `useAxiomQuery<${returnType}>(factory(args), options)` : `useAxiomMutation<${returnType}, ${argType}>(factory)`};
  }\n`;
}
function generateQueryDefFactory(ep, returnType, ns) {
    const params = ep.parameters || [];
    const bodyParam = params.find((p) => p.source === "body");
    const payloadLogic = bodyParam
        ? `const payload = (args as any).${(0, utils_1.camelCase)(bodyParam.name)};`
        : `const payload = undefined;`;
    const decLogic = generateLambda(ep.returnType, "fromJson", ns);
    const serLogic = bodyParam
        ? generateLambda(bodyParam.typeRef, "toJson", ns)
        : `(p: any) => p`;
    // THIN DEFINITION: Leaves URL logic up to ATMX Core Router
    return `(args: any): AxiomQueryDef<${returnType}> => {
      ${payloadLogic}
      return {
        namespace: "${ns}",
        name: "${ep.name}",
        endpointId: ${ep.id},
        method: "${ep.method ? ep.method.toUpperCase() : "GET"}",
        path: "${ep.path}",
        payload: payload,
        args: args as any,
        decoder: ${decLogic},
        serializer: ${serLogic},
        isStream: ${ep.isStream === true}
      };
    }`;
}
function generateLambda(typeRef, mode, ns) {
    if (!typeRef || !typeRef.kind || typeRef.kind === "void")
        return mode === "fromJson" ? `() => undefined` : `(p: any) => p`;
    if (typeRef.kind === "list" && typeRef.value?.kind === "named")
        return `(data: any[]) => data.map(models.Mappers.${(0, utils_1.camelCase)(ns)}.${(0, utils_1.pascalCase)(typeRef.value.value)}.${mode})`;
    if (typeRef.kind === "named")
        return `models.Mappers.${(0, utils_1.camelCase)(ns)}.${(0, utils_1.pascalCase)(typeRef.value)}.${mode}`;
    return `(data: any) => data`;
}
function prefixModels(type) {
    if (!type || type === "void" || type === "any")
        return type;
    if (type.endsWith("[]"))
        return `${prefixModels(type.slice(0, -2))}[]`;
    if (type.startsWith("models."))
        return type;
    return `models.${type}`;
}
