// atmx-cli/src/generators/utils.ts

export function pascalCase(str: string): string {
  if (!str) return "";
  return str
    .split(/[_\-\s]+/)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join("");
}

export function camelCase(str: string): string {
  const pascal = pascalCase(str);
  return pascal.charAt(0).toLowerCase() + pascal.slice(1);
}

export function normalizeIr(obj: any): any {
  // ✨ FIX: Arrays in JavaScript are Objects! We must check Array.isArray FIRST.
  if (Array.isArray(obj)) {
    return obj.map(normalizeIr);
  }

  if (obj !== null && typeof obj === "object") {
    const newObj: any = {};
    for (const key of Object.keys(obj)) {
      const camelKey = key.replace(/_([a-z])/g, (g) => g[1].toUpperCase());
      newObj[camelKey] = normalizeIr(obj[key]);
    }

    // Convert common Maps to Arrays if they exist and are not already arrays
    if (
      newObj.endpoints &&
      typeof newObj.endpoints === "object" &&
      !Array.isArray(newObj.endpoints)
    ) {
      newObj.endpoints = Object.values(newObj.endpoints);
    }
    if (
      newObj.models &&
      typeof newObj.models === "object" &&
      !Array.isArray(newObj.models)
    ) {
      newObj.models = Object.values(newObj.models);
    }
    if (
      newObj.enums &&
      typeof newObj.enums === "object" &&
      !Array.isArray(newObj.enums)
    ) {
      newObj.enums = Object.values(newObj.enums);
    }

    // Traverse down into models to normalize fields
    if (Array.isArray(newObj.models)) {
      newObj.models = newObj.models.map((model: any) => {
        if (
          model.fields &&
          typeof model.fields === "object" &&
          !Array.isArray(model.fields)
        ) {
          model.fields = Object.values(model.fields);
        }
        return model;
      });
    }

    return newObj;
  }

  return obj;
}

/**
 * @param scopedNamespace If provided (e.g. 'Auth'), prefixes named types with 'models.Auth.'
 */
export function mapTypeToTs(typeRef: any, ns?: string): string {
  if (!typeRef) return "any";

  if (typeRef.kind === "primitive") {
    switch (typeRef.value) {
      case "string":
        return "string";
      case "int":
      case "float":
      case "double":
        return "number";
      case "boolean":
        return "boolean";
      case "dateTime":
        return "Date";
      case "bytes":
        return "Uint8Array";
      default:
        return "any";
    }
  }

  if (typeRef.kind === "named") {
    const name = pascalCase(typeRef.value);
    return ns ? `${ns}${name}` : name;
  }

  if (typeRef.kind === "list") {
    return `${mapTypeToTs(typeRef.value, ns)}[]`;
  }

  return "any";
}
