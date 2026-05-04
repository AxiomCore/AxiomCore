"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.pascalCase = pascalCase;
exports.camelCase = camelCase;
exports.normalizeIr = normalizeIr;
exports.mapTypeToTs = mapTypeToTs;
// FILE: atmx-cli/src/generators/utils.ts
function pascalCase(str) {
    if (!str)
        return "";
    return str
        .split(/[_\-\s]+/)
        .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
        .join("");
}
function camelCase(str) {
    const pascal = pascalCase(str);
    return pascal.charAt(0).toLowerCase() + pascal.slice(1);
}
function normalizeIr(obj) {
    if (Array.isArray(obj))
        return obj.map(normalizeIr);
    if (obj !== null && typeof obj === "object") {
        const newObj = {};
        for (const key of Object.keys(obj)) {
            const camelKey = key.replace(/_([a-z])/g, (g) => g[1].toUpperCase());
            newObj[camelKey] = normalizeIr(obj[key]);
        }
        if (newObj.endpoints &&
            typeof newObj.endpoints === "object" &&
            !Array.isArray(newObj.endpoints)) {
            newObj.endpoints = Object.values(newObj.endpoints);
        }
        if (newObj.models &&
            typeof newObj.models === "object" &&
            !Array.isArray(newObj.models)) {
            newObj.models = Object.values(newObj.models);
        }
        if (newObj.enums &&
            typeof newObj.enums === "object" &&
            !Array.isArray(newObj.enums)) {
            newObj.enums = Object.values(newObj.enums);
        }
        if (Array.isArray(newObj.models)) {
            newObj.models = newObj.models.map((model) => {
                if (model.fields &&
                    typeof model.fields === "object" &&
                    !Array.isArray(model.fields)) {
                    model.fields = Object.values(model.fields);
                }
                return model;
            });
        }
        return newObj;
    }
    return obj;
}
function mapTypeToTs(typeRef, ns) {
    if (!typeRef || !typeRef.kind)
        return "any";
    switch (typeRef.kind) {
        case "string":
            return "string";
        case "int32":
        case "int64":
        case "float32":
        case "float64":
            return "number";
        case "bool":
            return "boolean";
        case "dateTime":
            return "Date";
        case "bytes":
            return "Uint8Array";
        case "void":
            return "void";
        case "json":
            return "any";
        case "named":
            const name = pascalCase(typeRef.value);
            return ns ? `${ns}.${name}` : name;
        case "list":
            return `${mapTypeToTs(typeRef.value, ns)}[]`;
        case "map":
            const valType = typeRef.value?.[1]
                ? mapTypeToTs(typeRef.value[1], ns)
                : "any";
            return `Record<string, ${valType}>`;
        default:
            return "any";
    }
}
