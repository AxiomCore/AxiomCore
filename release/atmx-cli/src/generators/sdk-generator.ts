import { AxiomIR, AxiomEndpoint, AxiomTypeRef } from "../types.js";

export interface ContractPayload {
  ir: AxiomIR;
  baseUrl: string;
  file: string;
}

// Helper to convert Axiom TypeRef to TypeScript Types
function getTsType(namespace: string, typeRef?: AxiomTypeRef): string {
  if (!typeRef) return "any";
  if (typeRef.kind === "named") {
    return `models.${namespace}.${typeRef.value}`;
  } else if (typeRef.kind === "list") {
    return `${getTsType(namespace, typeRef.value as AxiomTypeRef)}[]`;
  } else if (typeRef.kind === "primitive") {
    if (["int32", "int64", "float32", "float64"].includes(typeRef.value))
      return "number";
    if (typeRef.value === "bool") return "boolean";
    if (typeRef.value === "string") return "string";
  } else if (typeRef.kind === "void") {
    return "void";
  }
  return "any";
}

// Helper to get the correct deserializer function for a given TypeRef
function getDecoder(namespace: string, typeRef?: AxiomTypeRef): string {
  if (!typeRef) return `(json: any) => json`;

  if (typeRef.kind === "named") {
    return `models.Mappers.${namespace}.${typeRef.value}.fromJson`;
  } else if (
    typeRef.kind === "list" &&
    (typeRef.value as AxiomTypeRef).kind === "named"
  ) {
    const innerName = (typeRef.value as any).value;
    return `(json: any[]) => json.map(models.Mappers.${namespace}.${innerName}.fromJson)`;
  }

  return `(json: any) => json`;
}

export function generateSDKContent(
  contracts: Record<string, ContractPayload>,
  isReact: boolean,
): string {
  let content = `// GENERATED CODE – DO NOT EDIT.\n/* eslint-disable @typescript-eslint/no-explicit-any */\n/* eslint-disable @typescript-eslint/no-unused-vars */\n\n`;

  if (!isReact) {
    content += `import * as models from './models.js';\n\n`;
  } else {
    content += `import * as models from './models.js';\n`;
    content += `import { useAxiomQuery, useAxiomMutation, setAuthToken, clearAuthToken, axiomQueryManager } from "atmx-react";\n`;
    content += `import type { AxiomQueryDef } from "atmx-react";\n\n`;
  }

  // 1. Generate Individual Modules
  for (const [namespace, contract] of Object.entries(contracts)) {
    content += `export const ${namespace}Module = {\n`;
    content += `  axiom: {\n`;
    content += `    setAuthToken(methodName: string, token: string) {\n`;
    if (isReact) {
      content += `      setAuthToken("${namespace}", methodName, token);\n`;
    } else {
      content += `      (window as any).atmx?.setAuthToken("${namespace}", methodName, token);\n`;
    }
    content += `    },\n`;
    content += `    clearAuthToken(methodName: string) {\n`;
    if (isReact) {
      content += `      clearAuthToken("${namespace}", methodName);\n`;
    } else {
      content += `      (window as any).atmx?.clearAuthToken("${namespace}", methodName);\n`;
    }
    content += `    },\n`;
    content += `    connect(methodName: string, args?: Record<string, any>) {\n`;
    if (isReact) {
      content += `      const def = (${namespace}Module as any)[\`get\${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def\`](args);\n`;
      content += `      axiomQueryManager.connect(def);\n`;
    } else {
      content += `      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';\n`;
      content += `      (window as any).atmx?.connect(\`${namespace}.\${methodName}(\${argsStr})\`);\n`;
    }
    content += `    },\n`;
    content += `    disconnect(methodName: string, args?: Record<string, any>) {\n`;
    if (isReact) {
      content += `      const def = (${namespace}Module as any)[\`get\${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def\`](args);\n`;
      content += `      axiomQueryManager.disconnect(def);\n`;
    } else {
      content += `      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';\n`;
      content += `      (window as any).atmx?.disconnect(\`${namespace}.\${methodName}(\${argsStr})\`);\n`;
    }
    content += `    },\n`;
    content += `    send(methodName: string, payload: any, args?: Record<string, any>) {\n`;
    if (isReact) {
      content += `      const def = (${namespace}Module as any)[\`get\${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def\`](args);\n`;
      content += `      axiomQueryManager.send(def, payload);\n`;
    } else {
      content += `      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';\n`;
      content += `      (window as any).atmx?.send(\`${namespace}.\${methodName}(\${argsStr})\`, payload);\n`;
    }
    content += `    }\n`;
    content += `  },\n\n`;

    // Extract endpoints handling objects vs arrays
    const endpoints: AxiomEndpoint[] = Array.isArray(contract.ir.endpoints)
      ? contract.ir.endpoints
      : Object.values(contract.ir.endpoints || {});

    for (const endpoint of endpoints) {
      const fnName = endpoint.name.replace(/_([a-z])/g, (g) =>
        g[1].toUpperCase(),
      );
      const capFnName = fnName.charAt(0).toUpperCase() + fnName.slice(1);

      if (isReact) {
        const tsType = getTsType(namespace, endpoint.returnType);
        const decoder = getDecoder(namespace, endpoint.returnType);

        content += `  get${capFnName}Def(\n`;
        content += `    args?: Record<string, any>,\n`;
        content += `  ): AxiomQueryDef<${tsType}> {\n`;
        content += `    return {\n`;
        content += `      namespace: "${namespace}",\n`;
        content += `      name: "${endpoint.name}",\n`;
        content += `      endpointId: ${endpoint.id || 0},\n`;
        content += `      method: "${endpoint.method}",\n`;
        content += `      path: "${endpoint.path}",\n`;
        content += `      args: args || {},\n`;
        content += `      decoder: ${decoder},\n`;
        content += `      serializer: (p: any) => p,\n`;
        content += `      isStream: ${endpoint.isStream ? "true" : "false"},\n`;
        content += `    };\n`;
        content += `  },\n`;

        if (endpoint.method === "GET" || endpoint.method === "WS") {
          content += `  use${capFnName}(options?: { enabled?: boolean }) {\n`;
          content += `    return useAxiomQuery<${tsType}>(\n`;
          content += `      this.get${capFnName}Def(),\n`;
          content += `      options,\n`;
          content += `    );\n`;
          content += `  },\n`;
        } else {
          content += `  use${capFnName}(options?: any) {\n`;
          content += `    return useAxiomMutation<${tsType}>(\n`;
          content += `      this.get${capFnName}Def(),\n`;
          content += `      options,\n`;
          content += `    );\n`;
          content += `  },\n`;
        }
      } else {
        // Vanilla Web generator logic
        content += `  ${fnName}(args?: Record<string, any>): string {\n`;
        content += `    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';\n`;
        content += `    return \`${namespace}.${endpoint.name}(\${argsStr})\`;\n`;
        content += `  },\n`;
      }
    }
    content += `};\n\n`;
  }

  // 2. Generate the Smart Proxy SDK
  content += `const internalSdk: Record<string, any> = {\n`;
  for (const namespace of Object.keys(contracts)) {
    content += `  ${namespace}: ${namespace}Module,\n`;
  }
  content += `};\n\n`;

  content += `// ✨ The Magic Proxy: Safely intercepts Alpine.js evaluations during boot!\n`;
  content += `export const sdk = new Proxy(internalSdk, {\n`;
  content += `  get(target: any, prop: string, receiver: any) {\n`;
  content += `    if (prop in target) {\n`;
  content += `      return Reflect.get(target, prop, receiver);\n`;
  content += `    }\n`;
  content += `    // If Alpine tries to access a namespace that doesn't exist yet, return a nested Proxy!\n`;
  content += `    return new Proxy({}, {\n`;
  content += `      get(subTarget: any, subProp: string) {\n`;
  content += `        return () => \`\${String(prop)}.\${String(subProp)}()\`;\n`;
  content += `      }\n`;
  content += `    });\n`;
  content += `  }\n`;
  content += `});\n\n`;

  content += `// Auto-attach to window for Alpine.js immediate hydration\n`;
  content += `if (typeof window !== "undefined") {\n`;
  content += `  (window as any).sdk = sdk;\n`;
  content += `}\n\n`;

  // 3. Generate Default Config
  content += `export const AxiomDefaultConfig = {\n`;
  content += `  contracts: {\n`;
  for (const [ns, def] of Object.entries(contracts)) {
    // Determine file path or default to namespace
    const contractPath = def.file ? def.file : `/${ns}.axiom`;

    content += `    "${ns}": {\n`;
    content += `      contractUrl: "${contractPath}",\n`;
    content += `      baseUrl: "${def.baseUrl}"\n`;
    content += `    },\n`;
  }
  content += `  }\n`;
  content += `};\n`;

  return content;
}
