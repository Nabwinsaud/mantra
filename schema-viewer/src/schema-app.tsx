import { useCallback, useEffect, useMemo, useState } from 'react';
import ELK from 'elkjs/lib/elk.bundled.js';
import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  MiniMap,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useEdgesState,
  useNodesInitialized,
  useNodesState,
  useReactFlow,
  type Edge,
  type Node,
  type NodeProps,
} from '@xyflow/react';
import {
  Columns3,
  Command,
  Database,
  Focus,
  KeyRound,
  LayoutDashboard,
  Link2,
  LoaderCircle,
  Search,
  Table2,
  X,
} from 'lucide-react';
import type {
  DiagramRelationship,
  DiagramTable,
  SchemaDiagram,
} from './types';
import {
  RelationshipEdge as RelationshipEdgeComponent,
  type RelationshipEdgeData,
} from './relationship-edge';

type TableNodeData = Record<string, unknown> & {
  table: DiagramTable;
  incoming: number;
  outgoing: number;
  parentColumns: string[];
  childColumns: string[];
};

type TableNode = Node<TableNodeData, 'databaseTable'>;
type RelationshipEdge = Edge<RelationshipEdgeData, 'relationship'>;

const elk = new ELK();
const NODE_WIDTH = 304;
const HEADER_HEIGHT = 64;
const COLUMN_HEIGHT = 28;
const NODE_FOOTER = 12;

const relationId = (schema: string, table: string) => `${schema}.${table}`;
const handleId = (role: 'parent' | 'child', column: string) =>
  `${role}:${column}`;

function DatabaseTableNode({ data, selected }: NodeProps<TableNode>) {
  const { table, incoming, outgoing, parentColumns, childColumns } = data;
  return (
    <article className={`table-node ${selected ? 'is-selected' : ''}`}>
      <header className="table-node__header">
        <div className="table-node__identity">
          <span className="table-node__schema">{table.schema}</span>
          <strong>{table.name}</strong>
        </div>
        <span className="table-node__kind">{table.kind}</span>
      </header>

      <div className="table-node__meta">
        <span>{table.columns.length} columns</span>
        <span>{incoming + outgoing} relations</span>
        {table.kind.includes('table') && (
          <span>~{table.estimated_rows.toLocaleString()} rows</span>
        )}
      </div>

      <div className="table-node__columns">
        {table.columns.map((column) => {
          const isParentColumn = parentColumns.includes(column.name);
          const isChildColumn = childColumns.includes(column.name);
          return (
          <div
            className={`column-row ${isParentColumn || isChildColumn ? 'is-related' : ''}`}
            key={column.name}
          >
            {isChildColumn && (
              <Handle
                id={handleId('child', column.name)}
                type="target"
                position={Position.Left}
                className="column-handle column-handle--child"
              />
            )}
            <div className="column-row__name">
              {column.primary_key && (
                <KeyRound aria-label="Primary key" className="column-key" size={13} />
              )}
              <span>{column.name}</span>
              {column.unique && !column.primary_key && (
                <span className="column-unique" title="Unique">UQ</span>
              )}
              {isChildColumn && <span className="column-foreign" title="Foreign key">FK</span>}
            </div>
            <div className="column-row__type">
              <span>{column.data_type}</span>
              {column.nullable && <i>nullable</i>}
            </div>
            {isParentColumn && (
              <Handle
                id={handleId('parent', column.name)}
                type="source"
                position={Position.Right}
                className="column-handle column-handle--parent"
              />
            )}
          </div>
          );
        })}
      </div>
    </article>
  );
}

const nodeTypes = { databaseTable: DatabaseTableNode };
const edgeTypes = { relationship: RelationshipEdgeComponent };

async function layoutDiagram(diagram: SchemaDiagram) {
  const counts = new Map<string, { incoming: number; outgoing: number }>();
  const parentColumns = new Map<string, Set<string>>();
  const childColumns = new Map<string, Set<string>>();
  for (const table of diagram.tables) {
    const id = relationId(table.schema, table.name);
    counts.set(id, { incoming: 0, outgoing: 0 });
    parentColumns.set(id, new Set());
    childColumns.set(id, new Set());
  }
  for (const relationship of diagram.relationships) {
    const source = relationId(relationship.source_schema, relationship.source_table);
    const target = relationId(relationship.target_schema, relationship.target_table);
    const sourceCount = counts.get(source);
    const targetCount = counts.get(target);
    if (sourceCount) sourceCount.outgoing += 1;
    if (targetCount) targetCount.incoming += 1;
    relationship.source_columns.forEach((column) => childColumns.get(source)?.add(column));
    relationship.target_columns.forEach((column) => parentColumns.get(target)?.add(column));
  }

  const graph = await elk.layout({
    id: 'root',
    layoutOptions: {
      'elk.algorithm': 'layered',
      'elk.direction': 'RIGHT',
      'elk.edgeRouting': 'ORTHOGONAL',
      'elk.layered.spacing.nodeNodeBetweenLayers': '132',
      'elk.spacing.nodeNode': '64',
      'elk.spacing.componentComponent': '92',
      'elk.layered.considerModelOrder.strategy': 'NODES_AND_EDGES',
    },
    children: diagram.tables.map((table) => ({
      id: relationId(table.schema, table.name),
      width: NODE_WIDTH,
      height: HEADER_HEIGHT + table.columns.length * COLUMN_HEIGHT + NODE_FOOTER,
    })),
    edges: diagram.relationships.map((relationship, index) => ({
      id: `${relationship.name}-${index}`,
      sources: [relationId(relationship.target_schema, relationship.target_table)],
      targets: [relationId(relationship.source_schema, relationship.source_table)],
    })),
  });

  const positions = new Map(
    graph.children?.map((node) => [node.id, { x: node.x ?? 0, y: node.y ?? 0 }]),
  );
  const nodes: TableNode[] = diagram.tables.map((table) => {
    const id = relationId(table.schema, table.name);
    const count = counts.get(id) ?? { incoming: 0, outgoing: 0 };
    const height = HEADER_HEIGHT + table.columns.length * COLUMN_HEIGHT + NODE_FOOTER;
    return {
      id,
      type: 'databaseTable',
      position: positions.get(id) ?? { x: 0, y: 0 },
      initialWidth: NODE_WIDTH,
      initialHeight: height,
      data: {
        table,
        ...count,
        parentColumns: [...(parentColumns.get(id) ?? [])],
        childColumns: [...(childColumns.get(id) ?? [])],
      },
    };
  });
  const edges: RelationshipEdge[] = diagram.relationships.map((relationship, index) => ({
    id: `${relationship.name}-${index}`,
    source: relationId(relationship.target_schema, relationship.target_table),
    target: relationId(relationship.source_schema, relationship.source_table),
    sourceHandle: handleId('parent', relationship.target_columns[0] ?? ''),
    targetHandle: handleId('child', relationship.source_columns[0] ?? ''),
    type: 'relationship',
    data: { relationship },
  }));
  return { nodes, edges };
}

function SchemaCanvas({ diagram }: { diagram: SchemaDiagram }) {
  const [nodes, setNodes, onNodesChange] = useNodesState<TableNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<RelationshipEdge>([]);
  const [query, setQuery] = useState('');
  const [schema, setSchema] = useState('all');
  const [selectedTableId, setSelectedTableId] = useState<string | null>(null);
  const [layoutBusy, setLayoutBusy] = useState(true);
  const { fitView } = useReactFlow();
  const nodesInitialized = useNodesInitialized();

  const schemas = useMemo(
    () => [...new Set(diagram.tables.map((table) => table.schema))].sort(),
    [diagram.tables],
  );
  const selectedTable = diagram.tables.find(
    (table) => relationId(table.schema, table.name) === selectedTableId,
  );
  const selectedRelationships = selectedTableId
    ? diagram.relationships.filter(
        (relationship) =>
          relationId(relationship.source_schema, relationship.source_table) === selectedTableId ||
          relationId(relationship.target_schema, relationship.target_table) === selectedTableId,
      )
    : [];

  const performLayout = useCallback(async () => {
    setLayoutBusy(true);
    const layout = await layoutDiagram(diagram);
    setNodes(layout.nodes);
    setEdges(layout.edges);
    setLayoutBusy(false);
  }, [diagram, setEdges, setNodes]);

  useEffect(() => {
    void performLayout();
  }, [performLayout]);

  useEffect(() => {
    if (nodesInitialized && !layoutBusy && nodes.length > 0) {
      window.setTimeout(() => fitView({ padding: 0.12, duration: 500 }), 20);
    }
  }, [fitView, layoutBusy, nodes.length, nodesInitialized]);

  useEffect(() => {
    const normalized = query.trim().toLocaleLowerCase();
    setNodes((current) =>
      current.map((node) => {
        const table = node.data.table;
        const schemaMatches = schema === 'all' || table.schema === schema;
        const textMatches =
          !normalized ||
          `${table.schema}.${table.name}`.toLocaleLowerCase().includes(normalized) ||
          table.columns.some((column) =>
            `${column.name} ${column.data_type}`.toLocaleLowerCase().includes(normalized),
          );
        const visible = schemaMatches && textMatches;
        return { ...node, hidden: !schemaMatches, style: { opacity: visible ? 1 : 0.16 } };
      }),
    );
  }, [query, schema, setNodes]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === '/' && document.activeElement?.tagName !== 'INPUT') {
        event.preventDefault();
        document.querySelector<HTMLInputElement>('#schema-search')?.focus();
      } else if (event.key === 'Escape') {
        setQuery('');
        setSelectedTableId(null);
        document.querySelector<HTMLInputElement>('#schema-search')?.blur();
      } else if (event.key.toLowerCase() === 'f' && document.activeElement?.tagName !== 'INPUT') {
        void fitView({ padding: 0.12, duration: 400 });
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [fitView]);

  return (
    <div className="schema-shell">
      <header className="topbar">
        <div className="brand">
          <div className="brand__mark"><Database size={18} /></div>
          <div>
            <span>MANTRA</span>
            <strong>{diagram.database}</strong>
          </div>
        </div>

        <div className="search-box">
          <Search size={16} />
          <input
            id="schema-search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Find a table, column, or type"
            autoComplete="off"
          />
          <kbd>/</kbd>
        </div>

        <div className="topbar__actions">
          <button onClick={() => void fitView({ padding: 0.12, duration: 400 })}>
            <Focus size={15} /> Fit
          </button>
          <button onClick={() => void performLayout()} disabled={layoutBusy}>
            {layoutBusy ? <LoaderCircle className="spin" size={15} /> : <LayoutDashboard size={15} />}
            Layout
          </button>
        </div>
      </header>

      <div className="schema-strip">
        <div className="metric"><Table2 size={15} /><b>{diagram.tables.length}</b> tables</div>
        <div className="metric"><Columns3 size={15} /><b>{diagram.tables.reduce((sum, table) => sum + table.columns.length, 0)}</b> columns</div>
        <div className="metric"><Link2 size={15} /><b>{diagram.relationships.length}</b> foreign keys</div>
        <div className="metric relation-legend"><b>||</b> one <b>o&#123;</b> many</div>
        <div className="schema-filter" role="group" aria-label="Filter schemas">
          {['all', ...schemas].map((name) => (
            <button
              className={schema === name ? 'is-active' : ''}
              key={name}
              onClick={() => setSchema(name)}
            >
              {name}
            </button>
          ))}
        </div>
      </div>

      <main className="canvas-wrap">
        <ReactFlow<TableNode, RelationshipEdge>
          nodes={nodes}
          edges={edges.map((edge) => {
            const connected = !selectedTableId || edge.source === selectedTableId || edge.target === selectedTableId;
            return {
              ...edge,
              animated: Boolean(selectedTableId && connected),
              style: { opacity: connected ? 0.88 : 0.09 },
            };
          })}
          nodeTypes={nodeTypes}
          edgeTypes={edgeTypes}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onNodeClick={(_, node) => setSelectedTableId(node.id)}
          onPaneClick={() => setSelectedTableId(null)}
          nodesConnectable={false}
          edgesReconnectable={false}
          minZoom={0.08}
          maxZoom={2}
          proOptions={{ hideAttribution: true }}
          fitView
        >
          <Background color="#2a3042" gap={28} size={1} variant={BackgroundVariant.Dots} />
          {diagram.tables.length > 12 && (
            <MiniMap
              nodeColor={(node) => node.selected ? '#d8ae6a' : '#72a7ff'}
              maskColor="rgba(8, 10, 16, 0.78)"
              pannable
              zoomable
            />
          )}
          <Controls showInteractive={false} />
        </ReactFlow>

        <div className="canvas-hint">
          <span><Command size={13} /> / search</span>
          <span>F fit</span>
          <span>drag to arrange</span>
          <span>click a relation for details</span>
          <span>scroll to zoom</span>
        </div>

        {selectedTable && (
          <aside className="inspector">
            <button className="inspector__close" onClick={() => setSelectedTableId(null)} aria-label="Close inspector">
              <X size={17} />
            </button>
            <span className="eyebrow">{selectedTable.schema} · {selectedTable.kind}</span>
            <h2>{selectedTable.name}</h2>
            <div className="inspector__stats">
              <span><b>{selectedTable.columns.length}</b> columns</span>
              <span><b>{selectedRelationships.length}</b> relations</span>
              <span><b>~{selectedTable.estimated_rows.toLocaleString()}</b> rows</span>
            </div>
            <section>
              <h3>Columns</h3>
              <div className="inspector__columns">
                {selectedTable.columns.map((column) => (
                  <div key={column.name}>
                    <strong>{column.name}</strong>
                    <span>{column.data_type}{column.nullable ? ' · nullable' : ''}</span>
                    {column.default && <code>{column.default}</code>}
                    {column.comment && <p>{column.comment}</p>}
                  </div>
                ))}
              </div>
            </section>
            <section>
              <h3>Relationships</h3>
              {selectedRelationships.length === 0 ? (
                <p className="empty-copy">No declared foreign keys.</p>
              ) : selectedRelationships.map((relationship) => (
                <div className="relationship" key={relationship.name}>
                  <b>{relationship.name}</b>
                  <span>
                    {relationship.source_schema}.{relationship.source_table}
                    <em>{relationship.source_columns.join(', ')}</em>
                  </span>
                  <span className="relationship__arrow">→</span>
                  <span>
                    {relationship.target_schema}.{relationship.target_table}
                    <em>{relationship.target_columns.join(', ')}</em>
                  </span>
                  <small>on delete {relationship.on_delete.toLocaleLowerCase()}</small>
                </div>
              ))}
            </section>
          </aside>
        )}
      </main>
    </div>
  );
}

function Viewer() {
  const [diagram, setDiagram] = useState<SchemaDiagram | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const base = window.location.pathname.replace(/\/$/, '');
    fetch(`${base}/schema`, { cache: 'no-store' })
      .then((response) => {
        if (!response.ok) throw new Error(`Schema request failed (${response.status})`);
        return response.json() as Promise<SchemaDiagram>;
      })
      .then(setDiagram)
      .catch((reason: unknown) =>
        setError(reason instanceof Error ? reason.message : 'Could not load schema metadata'),
      );
  }, []);

  if (error) {
    return <div className="center-state center-state--error"><Database size={28} /><h1>Schema unavailable</h1><p>{error}</p></div>;
  }
  if (!diagram) {
    return <div className="center-state"><LoaderCircle className="spin" size={28} /><h1>Mapping your database</h1><p>Reading tables, columns, keys, and relationships…</p></div>;
  }
  return <SchemaCanvas diagram={diagram} />;
}

export function SchemaApp() {
  return <ReactFlowProvider><Viewer /></ReactFlowProvider>;
}
