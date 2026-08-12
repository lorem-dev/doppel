// The part of JSON Schema this page reads.
//
// Not a general-purpose validator and deliberately not a library: what the form
// needs is the bound on one field -- a pattern, a length, a range -- from a
// document the server generated, and that is a walk down `properties`. The whole
// schema *is* validated as a schema, by ajv, but only where a whole document is
// the subject: the YAML editor. See `schema/rules.ts` for the walk and
// `store/schema.ts` for where the document comes from.

/** A schema node, as far as the field rules care. */
export interface JsonSchema {
  /** The document's own id, which is how a validator names its definitions. */
  $id?: string
  /** Which dialect it is written in. */
  $schema?: string
  $defs?: Record<string, JsonSchema>
  $ref?: string
  type?: string | string[]
  description?: string
  examples?: unknown[]
  properties?: Record<string, JsonSchema>
  required?: string[]
  /** A map's value schema, for `headers` and the variable maps. */
  additionalProperties?: JsonSchema | boolean
  /** A list's element schema, for `mocks`. */
  items?: JsonSchema
  /** How an `Option<T>` and a union of two representations both arrive. */
  oneOf?: JsonSchema[]
  pattern?: string
  minLength?: number
  maxLength?: number
  minimum?: number
  maximum?: number
  enum?: unknown[]
}
