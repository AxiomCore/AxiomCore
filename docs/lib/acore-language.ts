import type { LanguageRegistration } from '@shikijs/types';

/**
 * TextMate grammar for documentation examples. Keep keywords synchronized with
 * `acore/src/lexer.rs` and the Acore UI declarations parsed by `axiom-ui`.
 */
export const acoreLanguage: LanguageRegistration = {
  name: 'acore',
  displayName: 'Acore',
  scopeName: 'source.acore',
  fileTypes: ['acore'],
  patterns: [
    { include: '#comments' },
    { include: '#strings' },
    { include: '#numbers' },
    { include: '#constants' },
    { include: '#annotations' },
    { include: '#declarations' },
    { include: '#keywords' },
    { include: '#types' },
    { include: '#operators' },
  ],
  repository: {
    comments: {
      patterns: [
        { name: 'comment.line.double-slash.acore', match: '//.*$' },
      ],
    },
    strings: {
      patterns: [
        {
          name: 'string.quoted.triple.acore',
          begin: '"""',
          end: '"""',
          patterns: [{ include: '#escapes' }],
        },
        {
          name: 'string.quoted.double.acore',
          begin: '"',
          end: '"',
          patterns: [{ include: '#escapes' }],
        },
        {
          name: 'string.quoted.raw.acore',
          begin: '#"',
          end: '"#',
        },
      ],
    },
    escapes: {
      patterns: [
        { name: 'constant.character.escape.acore', match: '\\\\.' },
        {
          name: 'meta.interpolation.acore',
          begin: '\\\\?\\(',
          end: '\\)',
          patterns: [{ include: '$self' }],
        },
      ],
    },
    numbers: {
      patterns: [
        { name: 'constant.numeric.float.acore', match: '\\b-?\\d+\\.\\d+\\b' },
        { name: 'constant.numeric.integer.acore', match: '\\b-?\\d+\\b' },
      ],
    },
    constants: {
      patterns: [
        { name: 'constant.language.acore', match: '\\b(?:true|false|null)\\b' },
      ],
    },
    annotations: {
      patterns: [
        { name: 'storage.type.annotation.acore', match: '@[A-Za-z_][A-Za-z0-9_]*' },
      ],
    },
    declarations: {
      patterns: [
        {
          match: '\\b(module|class|typealias|function|app|page|state|action|query|mutation)\\s+([A-Za-z_][A-Za-z0-9_.]*)',
          captures: {
            1: { name: 'keyword.declaration.acore' },
            2: { name: 'entity.name.type.acore' },
          },
        },
      ],
    },
    keywords: {
      patterns: [
        {
          name: 'keyword.control.acore',
          match: '\\b(?:if|else|for|when|let|throw|trace|is|as)\\b',
        },
        {
          name: 'keyword.other.acore',
          match: '\\b(?:import|use|contract|theme|component|from|export|audience|appearance|read|amends|extend|extends|external|abstract|open|local|hidden|const|fixed|this|outer|super|view|route|on_press)\\b',
        },
      ],
    },
    types: {
      patterns: [
        {
          name: 'storage.type.acore',
          match: '\\b(?:Any|Null|Boolean|Int|Float|Number|String|Duration|DataSize|List|Listing|Set|Map|Mapping|Object|Dynamic|Module|Class|Function)\\b',
        },
        {
          name: 'entity.name.type.acore',
          match: '\\b[A-Z][A-Za-z0-9_]*\\b',
        },
      ],
    },
    operators: {
      patterns: [
        {
          name: 'keyword.operator.acore',
          match: '\\?\\.|\\?\\?|!!|\\|>|->|\\*\\*|~/|&&|\\|\\||==|!=|<=|>=|[=+\\-*/%<>!?:|]',
        },
      ],
    },
  },
};
