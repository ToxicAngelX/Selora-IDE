// Selora IDE — Phase 9a live todo spine (ROADMAP §"Phase 9", task 9a).
//
// Read-only view of the session's authoritative todo list: `ChatStateDto.todos`
// on hydrate and the real `manage_todo` tool event while a turn runs. When the
// list is absent the spine says "No todos yet" — it never seeds steps from the
// prompt or from markdown in the assistant's prose.
import React from 'react';
import type { AgentTodo } from '../types';

export type AgentTodoSpineProps = {
  todos: AgentTodo[];
  inFlight?: boolean;
};

export const AgentTodoSpine: React.FC<AgentTodoSpineProps> = ({ todos, inFlight = false }) => {
  const completed = todos.filter((todo) => todo.status === 'completed').length;
  const progress = todos.length > 0 ? Math.round((completed / todos.length) * 100) : 0;

  return (
    <section className="agent-todo-spine" aria-label="Session todos">
      <header className="agent-todo-spine-header">
        <h2 className="agent-todo-spine-title">Todos</h2>
        <span className="agent-todo-spine-count" aria-label={`${completed} of ${todos.length} complete`}>
          {todos.length > 0 ? `${completed}/${todos.length}` : '0'}
        </span>
        {inFlight ? (
          <span className="agent-todo-spine-live" role="status">
            Live
          </span>
        ) : null}
      </header>

      <div
        className="agent-todo-spine-track"
        role="progressbar"
        aria-label={`Todo progress: ${progress}%`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={progress}
      >
        <div className="agent-todo-spine-fill" style={{ width: `${progress}%` }} />
      </div>

      {todos.length === 0 ? (
        <p className="agent-todo-spine-empty">No todos yet</p>
      ) : (
        <ol className="agent-todo-spine-list">
          {todos.map((todo) => (
            <li
              key={todo.id}
              className={`agent-todo-spine-item is-${todo.status}`}
              data-status={todo.status}
            >
              <span className="agent-todo-spine-glyph" aria-hidden="true">
                {todo.status === 'completed' ? '✓' : todo.status === 'in_progress' ? '›' : '○'}
              </span>
              <span className="agent-todo-spine-text">{todo.title}</span>
              <span className="visually-hidden">
                {todo.status === 'completed'
                  ? 'completed'
                  : todo.status === 'in_progress'
                    ? 'in progress'
                    : 'pending'}
              </span>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
};