export type DiagramColumn = {
  name: string;
  data_type: string;
  nullable: boolean;
  default: string | null;
  comment: string | null;
  primary_key: boolean;
  unique: boolean;
};

export type DiagramTable = {
  schema: string;
  name: string;
  kind: string;
  estimated_rows: number;
  columns: DiagramColumn[];
};

export type DiagramRelationship = {
  name: string;
  source_schema: string;
  source_table: string;
  source_columns: string[];
  target_schema: string;
  target_table: string;
  target_columns: string[];
  source_optional: boolean;
  source_unique: boolean;
  on_update: string;
  on_delete: string;
};

export type SchemaDiagram = {
  database: string;
  tables: DiagramTable[];
  relationships: DiagramRelationship[];
};
