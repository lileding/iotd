# IoTD Development Roadmap

## Overview

IoTD (IoT Daemon) development follows a progressive milestone approach, where each milestone builds upon the previous one, adding new features while maintaining backward compatibility. This incremental approach ensures a stable foundation while continuously expanding capabilities.

## Development Milestones

### Milestone 1: Full MQTTv3 Server (QoS=0, no persistency/auth) ✅ **COMPLETED**
**Target**: Complete MQTT v3.1.1 server with QoS=0 support

**✅ Completed Features:**
- ✅ Event-driven architecture with tokio::select! based packet handling
- ✅ Race-condition-free shutdown using CancellationToken across all components
- ✅ Half-connected session tracking with proper cleanup
- ✅ Stream deadlock prevention in packet handlers
- ✅ UNIX signal handling (SIGINT graceful shutdown, SIGTERM immediate exit)
- ✅ Multi-broker architecture with Server → Broker → Session hierarchy
- ✅ Session management with sessionId lifecycle (`__anon_$uuid` → `__client_$clientId`)
- ✅ Thread-safe operations using DashMap and Arc
- ✅ Transport abstraction layer for multiple protocols
- ✅ Comprehensive configuration system with TOML support
- ✅ Complete MQTT v3.1.1 packet handling (all packet types tested)
- ✅ **Message routing system** with topic filtering and MQTT wildcard support (`+`, `#`)
- ✅ **Clean session logic** with session takeover and DISCONNECT notifications
- ✅ **Keep-alive mechanism** with configurable timeouts and ping/pong handling
- ✅ **Retained messages** with update/delete support, wildcard matching, and configurable limits
- ✅ **Will messages** (store on CONNECT, deliver on abnormal disconnect, clear on DISCONNECT)
- ✅ **Protocol compliance** (validation, error codes, client ID rules, topic validation)
- ✅ Comprehensive test suite (74 tests: 36 unit tests, 29 integration tests, 9 packet tests)

**Architecture Status:**
- ✅ CancellationToken-based shutdown eliminates race conditions
- ✅ Half-connected session tracking until CONNECT received
- ✅ Stream passing to packet handlers prevents deadlocks
- ✅ Thread-safe cleanup using lock-based swap pattern
- ✅ Atomic operations for performance optimization

**Timeline**: Completed!
**Success Criteria**:
- [✓] All MQTT v3.1.1 packet types working
- [✓] Message routing between clients functional
- [✓] Clean session behavior implemented
- [✓] Retained messages working
- [✓] Will messages working
- [✓] Keep-alive timeouts handled
- [✓] Full protocol compliance

---

### Milestone 2: QoS=1 Support (in-memory) ✅ **COMPLETED**
**Target**: QoS=1 message delivery with in-memory persistence

**✅ Completed Features:**
- ✅ QoS=1 (At least once) message delivery guarantee
- ✅ Message acknowledgment (PUBACK) handling
- ✅ Message retransmission with configurable intervals
- ✅ In-memory tracking of unacknowledged messages
- ✅ Duplicate message detection and handling (DUP flag)
- ✅ Multiple concurrent in-flight messages
- ✅ Packet ID management with sequential generation (1-65535)
- ✅ QoS downgrade (min of publish QoS and subscription QoS)
- ✅ Maximum retry limit enforcement
- ✅ Comprehensive test coverage (10+ QoS=1 specific tests)
- ✅ Performance validation under load

**Architecture Implemented:**
- Event-driven message handling with 4 key events
- VecDeque for efficient in-flight message queue
- Direct response pattern for reduced latency
- MQTT 3.1.1 specification compliant (no ordering, no exponential backoff)
- Support for multiple publishers and subscribers

**Timeline**: Completed in 2 weeks
**Success Criteria**: All achieved ✓
- [✓] QoS=1 messages delivered at least once
- [✓] Proper handling of duplicate messages
- [✓] Message retransmission on timeout
- [✓] Performance maintained under load
- [✓] Full MQTT 3.1.1 compliance

---

### Milestone 3: Persistence Layer 💾 ✅ **COMPLETED**
**Target**: Persistent storage for messages and sessions

**✅ Completed Features:**
- ✅ Unified Storage trait with pluggable backends
- ✅ InMemoryStorage for development/testing
- ✅ SqliteStorage for production persistence
- ✅ Session state persistence (clean_session=false)
- ✅ Subscription persistence across reconnects
- ✅ In-flight message persistence for QoS=1
- ✅ Retained message persistence
- ✅ Atomic session state save (all-or-nothing)
- ✅ Config-based storage backend selection
- ✅ Session restoration with CONNACK session_present flag

**Architecture Implemented:**
- Storage trait abstraction for pluggable backends
- Atomic save_session for session + subscriptions + inflight
- Router delegates retained messages to Storage
- Broker creates storage and passes to components

**Timeline**: Completed
**Success Criteria**: All achieved ✓
- [✓] Messages and sessions survive server restarts
- [✓] Storage backend configurable (memory/sqlite)
- [✓] Acceptable performance with persistence
- [✓] Reliable recovery mechanisms

---

### Milestone 4: Security (TLS, Auth, ACL) 🔐 ✅ **COMPLETED**
**Target**: TLS encryption, authentication, and authorization

**✅ Completed Features (4a - Authentication):**
- ✅ Username/password authentication
- ✅ File-based user credential management
- ✅ Pluggable auth backend (`allowall` and `file`)
- ✅ CONNECT packet credential validation
- ✅ Configurable auth via TOML

**✅ Completed Features (4b - ACLs):**
- ✅ Topic-based access control lists
- ✅ File-based ACL rule management
- ✅ Per-user and wildcard rules
- ✅ Publish and subscribe authorization
- ✅ Pluggable ACL backend (`allowall` and `file`)

**✅ Completed Features (4c - TLS):**
- ✅ TLS/SSL support via `tls://` listener prefix
- ✅ Multiple simultaneous listeners (TCP + TLS)
- ✅ Certificate and key file configuration
- ✅ tokio-rustls integration
- ✅ TLS integration tests with rcgen

**Architecture Implemented:**
- Multi-listener server spawning one task per address
- `AsyncStream::into_split()` for ownership-based stream splitting
- Backward-compatible config: `listen` accepts string or array
- TLS acceptor built once and shared across connections

**Timeline**: Completed
**Success Criteria**: All achieved ✓
- [✓] TLS encryption working properly
- [✓] Only authenticated clients can connect
- [✓] ACLs enforce topic access
- [✓] Performance acceptable with security enabled

---

### Milestone 5: QoS=2 Support 🎯 **PLANNED**
**Target**: Exactly-once delivery guarantee

**Features to Implement:**
- 🎯 QoS=2 (Exactly once) message delivery
- 🎯 PUBREC/PUBREL/PUBCOMP flow
- 🎯 Two-phase commit protocol
- 🎯 Message state persistence for QoS=2
- 🎯 Duplicate detection across restarts
- 🎯 Proper error handling and recovery

**Technical Challenges:**
- Complex state machine for QoS=2 flow
- Ensuring exactly-once semantics
- Performance impact of two-phase protocol
- Recovery after crashes

**Timeline**: 6-8 weeks
**Success Criteria**:
- [ ] QoS=2 messages delivered exactly once
- [ ] Proper handling of all edge cases
- [ ] State survives server restarts
- [ ] Acceptable performance

---

### Milestone 6: Observability & Monitoring 📊 **PLANNED**
**Target**: Production-grade monitoring with Grafana

**Features to Implement:**
- 📊 Prometheus metrics exporter
- 📊 Structured logging with levels
- 📊 Grafana dashboard templates
- 📊 Health check endpoints
- 📊 Connection and message metrics
- 📊 Performance metrics and tracing
- 📊 Alert rule templates

**Metrics to Export:**
- Connection count and rate
- Message throughput by QoS
- Topic statistics
- Error rates and types
- Resource usage (CPU, memory, disk)
- Latency percentiles

**Timeline**: 4-6 weeks
**Success Criteria**:
- [ ] Comprehensive metrics available
- [ ] Grafana dashboards working
- [ ] Alerts for common issues
- [ ] Performance impact minimal

---

### Milestone 7: Flow Control & Production Features 🚀 **PLANNED**
**Target**: Advanced features for production deployment

**Features to Implement:**
- 🚀 Per-client message rate limiting
- 🚀 In-flight message window limits
- 🚀 Connection throttling
- 🚀 Message size limits
- 🚀 WebSocket transport support
- 🚀 Unix domain socket support
- 🚀 Comprehensive documentation
- 🚀 Deployment guides and best practices

**Production Enhancements:**
- Resource usage controls
- DoS protection mechanisms
- Multi-protocol support
- Performance tuning guides
- Docker and Kubernetes manifests

**Timeline**: 6-8 weeks
**Success Criteria**:
- [ ] Flow control prevents resource exhaustion
- [ ] Multiple transport protocols working
- [ ] Production deployment guide complete
- [ ] Performance benchmarks documented

---

## Version Milestones

### v0.1.0 - MQTTv3 Foundation (Milestone 1) ✅ **COMPLETED**
- Complete MQTT v3.1.1 server ✓
- QoS 0 messaging with routing ✓
- Clean session with takeover support ✓
- Keep-alive mechanism ✓
- Retained messages with wildcard support ✓
- Will messages ✓
- Protocol compliance and validation ✓
- Event-driven architecture ✓

### v0.2.0 - Quality of Service (Milestone 2) ✅ **COMPLETED**
- QoS 1 support with acknowledgments ✓
- In-memory message tracking ✓
- Message retransmission logic ✓
- Duplicate detection with DUP flag ✓
- Multiple in-flight messages ✓
- Packet ID management (1-65535) ✓
- QoS downgrade handling ✓
- Comprehensive QoS=1 tests ✓

### v0.3.0 - Persistence (Milestone 3) 💾 ✅ **COMPLETED**
- Pluggable storage interface ✓
- Session state persistence ✓
- Retained message persistence ✓
- InMemory and SQLite backends ✓

### v0.4.0 - Security (Milestone 4) 🔐 ✅ **COMPLETED**
- TLS/SSL encryption ✓
- Username/password authentication ✓
- Topic-based ACLs ✓
- Multiple simultaneous listeners ✓

### v0.5.0 - QoS=2 (Milestone 5) 🎯
- Exactly-once delivery
- PUBREC/PUBREL/PUBCOMP flow
- Two-phase commit protocol
- State persistence for QoS=2

### v0.6.0 - Observability (Milestone 6) 📊
- Prometheus metrics
- Grafana dashboards
- Health check endpoints
- Structured logging

### v0.7.0 - Flow Control (Milestone 7) 🚀
- Rate limiting
- In-flight window limits
- WebSocket support
- Production features

### v1.0.0 - Production Ready 🏢
- Complete feature set
- Battle-tested stability
- Comprehensive documentation
- Performance optimized

---

## Future Versions (Post 1.0)

### v2.0 - MQTT 5.0 Support 🆕
**Target**: Full MQTT 5.0 protocol implementation

**Key Features:**
- Protocol version negotiation
- Enhanced authentication (AUTH packet)
- Shared subscriptions
- Message expiry intervals
- Request/Response pattern
- User properties
- Subscription identifiers
- Topic aliases for optimization
- Flow control (receive maximum)
- Server-initiated disconnect with reason codes
- Will delay intervals
- Maximum packet size negotiation

**Backward Compatibility:**
- Support both MQTT 3.1.1 and 5.0 clients
- Automatic protocol detection
- Graceful feature degradation

---

### v3.0 - Cluster Support 🌐
**Target**: Horizontal scalability and high availability

**Key Features:**
- Multi-node clustering
- Session migration between nodes
- Distributed topic tree
- Cluster-wide message routing
- Node discovery and auto-join
- Split-brain prevention
- Load balancing strategies
- Failover and recovery
- Distributed retained messages
- Cluster management API

**Architecture:**
- Raft consensus for cluster state
- Gossip protocol for node discovery
- Consistent hashing for topic distribution
- Active-active or active-passive modes

---

### v4.0 - Multi-tenancy & Soft Isolation 🏢
**Target**: Enterprise-grade multi-tenant deployment

**Key Features:**
- Tenant isolation at namespace level
- Per-tenant resource quotas
- Separate authentication per tenant
- Tenant-specific configurations
- Resource usage monitoring per tenant
- Cross-tenant communication policies
- Tenant management API
- Billing and metering support
- Data isolation guarantees
- Compliance and audit trails

**Use Cases:**
- SaaS MQTT broker offerings
- Enterprise internal shared infrastructure
- IoT platform providers
- Managed MQTT services

---

## Success Metrics

### Performance Targets
- **Connections**: 10,000+ concurrent clients
- **Throughput**: 100,000+ messages/second
- **Latency**: <1ms message routing
- **Memory**: <1MB per 1,000 idle connections
- **CPU**: <50% utilization at target load

### Reliability Targets
- **Uptime**: 99.9%+ availability
- **Message Loss**: <0.01% under normal conditions
- **Failover**: <5 second recovery time
- **Data Consistency**: ACID compliance for persistence

### Security Targets
- **Authentication**: Sub-100ms auth decisions
- **Authorization**: Sub-10ms permission checks
- **Encryption**: TLS 1.3 support
- **Compliance**: Common security standards

---

## Development Process

### Quality Assurance
- **Unit Tests**: >90% code coverage
- **Integration Tests**: End-to-end scenarios
- **Performance Tests**: Load and stress testing
- **Security Tests**: Vulnerability scanning
- **Compatibility Tests**: Multiple MQTT client libraries

### Documentation
- **Architecture Documentation**: Updated each milestone
- **API Documentation**: Comprehensive Rust docs
- **User Guide**: Installation and configuration
- **Developer Guide**: Contributing guidelines
- **Deployment Guide**: Production deployment

### Release Process
- **Feature Freeze**: 2 weeks before release
- **Beta Testing**: Community testing period
- **Security Review**: External security audit
- **Performance Validation**: Benchmark verification
- **Documentation Update**: Complete docs refresh

---

## Key Technical Decisions

### Architecture Evolution
- **Milestone 1**: Focus on correctness and basic functionality
- **Milestone 2+**: Add complexity incrementally with proper abstractions
- **Plugin System**: Designed from Milestone 6 for extensibility
- **Performance**: Optimized throughout with benchmarking

### Design Principles
- **Event-driven**: tokio::select! for responsive handling
- **Race-condition-free**: CancellationToken for reliable shutdown
- **Thread-safe**: DashMap and Arc for concurrent access
- **Modular**: Clean separation of concerns
- **Testable**: Comprehensive test coverage

### Technology Choices
- **Rust**: Memory safety and performance
- **Tokio**: Async runtime for high concurrency
- **CancellationToken**: Reliable shutdown coordination
- **DashMap**: High-performance concurrent HashMap
- **TOML**: Human-readable configuration

---

## What's Next: Milestone 5 - QoS=2 🎯

With security now complete, the next major milestone implements exactly-once delivery:

**Key Features to Implement:**
1. **QoS=2 Delivery**: PUBREC/PUBREL/PUBCOMP four-packet flow
2. **Two-phase commit**: Ensure exactly-once semantics
3. **State Persistence**: QoS=2 state survives server restarts
4. **Duplicate Detection**: Across reconnects and restarts
5. **Proper Error Handling**: Recovery after partial flows

**Preparation Tasks:**
- Extend session state machine for QoS=2 states
- Add QoS=2 in-flight tracking to storage trait
- Design packet ID reuse rules per MQTT 3.1.1 spec
- Plan comprehensive QoS=2 test scenarios

---

This roadmap provides a clear path toward building a production-ready, enterprise-grade MQTT broker while maintaining stability and performance at each milestone. The progressive approach ensures each milestone builds solid foundations for the next.