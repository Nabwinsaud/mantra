import {
  BaseEdge,
  EdgeLabelRenderer,
  getBezierPath,
  type EdgeProps,
} from '@xyflow/react';
import type { DiagramRelationship } from './types';

export type RelationshipEdgeData = Record<string, unknown> & {
  relationship: DiagramRelationship;
};

function CardinalityMark({
  side,
  many,
  optional,
  opacity,
  x,
  y,
}: {
  side: 'source' | 'target';
  many: boolean;
  optional: boolean;
  opacity: number;
  x: number;
  y: number;
}) {
  const direction = side === 'source' ? 1 : -1;
  const near = x + direction * 2;
  const middle = x + direction * 8;
  const far = x + direction * 14;

  return (
    <g className="cardinality-mark" opacity={opacity} aria-hidden="true">
      {many ? (
        <>
          <path d={`M ${near} ${y} L ${middle} ${y}`} />
          <path d={`M ${near} ${y} L ${middle} ${y - 4.5}`} />
          <path d={`M ${near} ${y} L ${middle} ${y + 4.5}`} />
        </>
      ) : (
        <path d={`M ${near} ${y - 4.5} L ${near} ${y + 4.5}`} />
      )}

      {optional ? (
        <circle cx={far} cy={y} r={3.2} />
      ) : (
        <path d={`M ${far} ${y - 4.5} L ${far} ${y + 4.5}`} />
      )}
    </g>
  );
}

export function RelationshipEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
  selected,
  style,
}: EdgeProps) {
  const relationship = (data as RelationshipEdgeData | undefined)?.relationship;
  const [path, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
    curvature: 0.32,
  });
  const sourceCardinality = relationship?.source_unique ? '0..1' : '0..many';
  const targetCardinality = relationship?.source_optional ? '0..1' : '1';
  const opacity = typeof style?.opacity === 'number' ? style.opacity : 1;

  return (
    <>
      <BaseEdge id={id} path={path} style={style} interactionWidth={22} />
      <CardinalityMark
        side="source"
        many={false}
        optional={Boolean(relationship?.source_optional)}
        opacity={opacity}
        x={sourceX}
        y={sourceY}
      />
      <CardinalityMark
        side="target"
        many={!relationship?.source_unique}
        optional
        opacity={opacity}
        x={targetX}
        y={targetY}
      />
      {relationship && selected && (
        <EdgeLabelRenderer>
          <div
            className="relationship-label is-selected"
            style={{
              opacity,
              transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
            }}
            title={`${relationship.name}: ${relationship.source_columns.join(', ')} references ${relationship.target_columns.join(', ')}`}
          >
            <span>{relationship.target_table}.{relationship.target_columns.join(', ')}</span>
            <b>→</b>
            <span>{relationship.source_table}.{relationship.source_columns.join(', ')}</span>
            <small>{targetCardinality} → {sourceCardinality}</small>
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  );
}
