// FILE: atmx-cli/src/generators/sdk-generator.ts
import { AxiomEndpoint, MultiIR } from "../types";
import { pascalCase, camelCase, mapTypeToTs } from "./utils";

export function generateSdk(multiIr: MultiIR): string {
  const lines: string[] = [
    `// GENERATED CODE – DO NOT EDIT.`,
    `import { useAxiomQuery, useAxiomMutation } from 'atmx-react';`,
    `import type { AxiomQueryDef } from 'atmx-react';`,
    `import * as models from './models';\n`,
  ];

  for (const [ns, ir] of Object.entries(multiIr)) {
    const pascalNs = pascalCase(ns);
    lines.push(`export class ${pascalNs}Module {`);
    const endpoints = ir.endpoints || [];
    endpoints.forEach((ep: any) => {
      lines.push(generateEndpointHook(ep, ns, pascalNs));
    });
    lines.push(`}\n`);
  }

  lines.push(`export class AxiomSdk {`);
  for (const ns of Object.keys(multiIr)) {
    lines.push(
      `  public readonly ${camelCase(ns)} = new ${pascalCase(ns)}Module();`,
    );
  }
  lines.push(`}\nexport const sdk = new AxiomSdk();`);

  return lines.join("\n");
}

function generateEndpointHook(
  ep: AxiomEndpoint,
  ns: string,
  pascalNs: string,
): string {
  const isQuery = ep.method ? ep.method.toUpperCase() === "GET" : true;
  const rawReturnType = mapTypeToTs(ep.returnType, pascalNs);
  const returnType =
    rawReturnType === "void" || rawReturnType === "any"
      ? rawReturnType
      : prefixModels(rawReturnType);

  const params = ep.parameters || [];
  const argType =
    params.length > 0
      ? `{ ${params.map((p) => `${camelCase(p.name)}${p.isOptional ? "?" : ""}: ${prefixModels(mapTypeToTs(p.typeRef, pascalNs))}`).join(", ")} }`
      : "void";

  const factoryLogic = generateQueryDefFactory(ep, returnType, ns);

  // NEW: Every endpoint now generates BOTH a raw definition getter and a hook!
  return `
  /** Raw definition for <AxQuery> or <AxMutate> */
  get${pascalCase(ep.name)}Def(args: ${argType}): AxiomQueryDef<${returnType}> {
    return (${factoryLogic})(args);
  }

  /** Reactive hook for functional components */
  use${pascalCase(ep.name)}${!isQuery ? "Mutation" : ""}(${isQuery ? `args: ${argType}, options?: { enabled?: boolean }` : ""}) {
    const factory = ${factoryLogic};
    return ${isQuery ? `useAxiomQuery<${returnType}>(factory(args), options)` : `useAxiomMutation<${returnType}, ${argType}>(factory)`};
  }\n`;
}

function generateQueryDefFactory(
  ep: AxiomEndpoint,
  returnType: string,
  ns: string,
): string {
  const params = ep.parameters || [];
  const bodyParam = params.find((p) => p.source === "body");
  const payloadLogic = bodyParam
    ? `const payload = (args as any).${camelCase(bodyParam.name)};`
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

function generateLambda(
  typeRef: any,
  mode: "fromJson" | "toJson",
  ns: string,
): string {
  if (!typeRef || !typeRef.kind || typeRef.kind === "void")
    return mode === "fromJson" ? `() => undefined` : `(p: any) => p`;
  if (typeRef.kind === "list" && typeRef.value?.kind === "named")
    return `(data: any[]) => data.map(models.Mappers.${camelCase(ns)}.${pascalCase(typeRef.value.value)}.${mode})`;
  if (typeRef.kind === "named")
    return `models.Mappers.${camelCase(ns)}.${pascalCase(typeRef.value)}.${mode}`;
  return `(data: any) => data`;
}

function prefixModels(type: string): string {
  if (!type || type === "void" || type === "any") return type;
  if (type.endsWith("[]")) return `${prefixModels(type.slice(0, -2))}[]`;
  if (type.startsWith("models.")) return type;
  return `models.${type}`;
}
