# Claude RAG Implementation Plan

> Phase-by-phase implementation roadmap

## Phase 1: Basic Framework

1. Create project structure
2. Implement Config module (read Claude Code config)
3. Implement CLI framework (clap)

## Phase 2: Data Parsing

4. Implement Parser module (scan session files)
5. Implement Models definitions (session/message/file/symbol/commit/diff structures) ⭐
6. Test data parsing functionality

## Phase 3: Embedding Service

7. Implement Embedding Client (Zhipu API)
8. Implement vector building logic (support multiple content types)
9. Test API calls

## Phase 4: Storage Layer (Core)

10. Implement sled wrapper (project-level database)
11. **Implement HNSW vector index**
    - Node and edge data structures
    - Insert algorithm
    - Search algorithm (ANN)
    - Persistence/loading
12. Implement semantic query functionality (support type filtering + Git content) ⭐

## Phase 5: File Scanning & Parsing

13. Implement FileScanner (project file scanning)
14. Implement .gitignore parser
15. Implement Tree-sitter AST parsing (source symbol extraction)
16. Implement document parser (Markdown segmentation, etc.)

## Phase 5a: Git History Collection ⭐

17. Implement GitCollector module (git2 wrapper)
18. Implement commit history parsing
19. Implement file diff extraction
20. Implement Conventional Commits parser

## Phase 6: Hook + Daemon

21. Implement session-start hook script generator
22. Implement daemon service
    - Unix socket / signal handling
    - Multi-project file monitoring (fsnotify)
    - Session timeout detection
    - Incremental indexing (sessions + source + docs)
23. Install hook to `~/.claude/hooks/`

## Phase 7: Time-Aware Retrieval ⭐

24. Implement confidence calculation module
25. Implement temporal decay calculator
26. Integrate temporal weighting into HNSW scoring
27. Implement timeline builder
28. Implement enhanced result structures
29. Implement result formatter with confidence badges
30. **Implement Git state synchronization with persistent caching** ⭐
    - Two-tier cache architecture (L1: Memory LRU + L2: Disk persistent)
    - HEAD change detection and cache invalidation
    - File content hashing (SHA-256) for modification detection
    - Background persistence task (5-minute interval)
    - Graceful degradation for persistence failures

## Phase 8: Claude Code Integration

30. **Implement Skills generator** ✅ Completed
    - Generate rag-query.sh, rag-code.sh, etc.
    - Install to `~/.claude/skills/`
31. **Implement MCP Server** ✅ Completed
    - MCP protocol implementation (stdio communication)
    - Implement tools: rag_query, rag_search_code, rag_search_docs, rag_search_session
    - Support time-aware queries (timeline, commit-specific)
32. Configure MCP to Claude Code ✅ Completed
33. **Plan Indexing** ✅ Completed
    - Plan model with title/content/chunk support
    - Four-level project matching strategy
    - PlanCollector with incremental collection
    - Storage integration (plan: prefix)

## Phase 9: Feature Refinement

33. Error handling and logging
34. Progress display (during indexing)
35. Testing and optimization

---

## Verification Plan

1. **Config Test**: Confirm correct reading of `~/.claude/rag/config.toml`
2. **Parsing Test**: Confirm ability to parse existing session files
3. **API Test**: Confirm successful Zhipu API calls
4. **Storage Test**:
   - Confirm sled database correctly created in project directory
   - Confirm HNSW index insertion and search work correctly
5. **Hook Test**: Confirm session-start hook can notify daemon
6. **Daemon Test**: Confirm ability to monitor and index new messages in real-time
7. **Query Test**: Confirm semantic retrieval returns relevant results

---

## Progress Tracking

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Basic Framework | 🟢 Completed | |
| Phase 2: Data Parsing | 🟢 Completed | |
| Phase 3: Embedding Service | 🟢 Completed | |
| Phase 4: Storage Layer | 🟢 Completed | Core dependency |
| Phase 5: File Scanning | 🟢 Completed | |
| Phase 5a: Git History | 🟢 Completed | ⭐ New feature |
| Phase 6: Hook + Daemon | 🟢 Completed | |
| Phase 7: Time-Aware Retrieval | 🟢 Completed | ⭐ GitSync with persistent caching implemented |
| Phase 8: Claude Code Integration | 🟢 Completed | ⭐ Plan indexing implemented |
| Phase 9: Feature Refinement | 🟢 Completed | |

---

*Legend: ⬜ Not Started | 🟡 In Progress | 🟢 Completed | 🔴 Blocked*
