"use strict";
// atmx-cli/src/generators/utils.ts
Object.defineProperty(exports, "__esModule", { value: true });
exports.pascalCase = pascalCase;
exports.camelCase = camelCase;
exports.normalizeIr = normalizeIr;
exports.mapTypeToTs = mapTypeToTs;
function pascalCase(str) {
    if (!str)
        return '';
    return str
        .split(/[_\-\s]+/)
        .map(part => part.charAt(0).toUpperCase() + part.slice(1))
        .join('');
}
function camelCase(str) {
    const pascal = pascalCase(str);
    return pascal.charAt(0).toLowerCase() + pascal.slice(1);
}
function normalizeIr(obj) {
    if (Array.isArray(obj))
        return obj.map(normalizeIr);
    if (obj !== null && typeof obj === 'object') {
        const newObj = {};
        for (const key of Object.keys(obj)) {
            const camelKey = key.replace(/_([a-z])/g, (g) => g[1].toUpperCase());
            newObj[camelKey] = normalizeIr(obj[key]);
        }
        return newObj;
    }
    return obj;
}
/**
 * @param scopedNamespace If provided (e.g. 'Auth'), prefixes named types with 'models.Auth.'
 */
function mapTypeToTs(typeRef, ns) {
    if (!typeRef)
        return 'any';
    if (typeRef.kind === 'primitive') {
        switch (typeRef.value) {
            case 'string': return 'string';
            case 'int':
            case 'float':
            case 'double': return 'number';
            case 'boolean': return 'boolean';
            case 'dateTime': return 'Date';
            case 'bytes': return 'Uint8Array';
            default: return 'any';
        }
    }
    if (typeRef.kind === 'named') {
        const name = pascalCase(typeRef.value);
        return ns ? `${ns}${name}` : name; // ✅ FIX HERE
    }
    if (typeRef.kind === 'list') {
        return `${mapTypeToTs(typeRef.value, ns)}[]`;
    }
    return 'any';
}
