# IoTHub Development Roadmap

## Overview

IoTHub development follows a progressive milestone approach, where each milestone builds upon the previous one, adding new features while maintaining backward compatibility. This incremental approach ensures a stable foundation while continuously expanding capabilities.

## Development Milestones

### Milestone 1: Full MQTTv3 Server (QoS=0, no persistency/auth) 🔄 **IN PROGRESS**
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
- ✅ Basic MQTT v3.1.1 packet handling (CONNECT, CONNACK, PUBLISH tested)

**🔄 In Progress:**
- 🔄 **Current**: Test all packet types (SUBSCRIBE, UNSUBSCRIBE, PINGREQ, DISCONNECT)

**❌ Missing Core Features:**
- ❌ **Message routing system** (biggest gap - route PUBLISH to subscribers)
- ❌ Clean session logic (handle `clean_session=false`)
- ❌ Retained messages (store and deliver to new subscribers)
- ❌ Will messages (store on CONNECT, deliver on abnormal disconnect)
- ❌ Keep-alive mechanism (monitor timeouts, cleanup expired sessions)
- ❌ Protocol compliance (proper error codes, client ID validation)

**Architecture Status:**
- ✅ CancellationToken-based shutdown eliminates race conditions
- ✅ Half-connected session tracking until CONNECT received
- ✅ Stream passing to packet handlers prevents deadlocks
- ✅ Thread-safe cleanup using lock-based swap pattern
- ✅ Atomic operations for performance optimization

**Timeline**: 4-6 weeks remaining
**Success Criteria**:
- [ ] All MQTT v3.1.1 packet types working
- [ ] Message routing between clients functional
- [ ] Clean session behavior implemented
- [ ] Retained and will messages working
- [ ] Keep-alive timeouts handled
- [ ] Full protocol compliance

---

### Milestone 2: QoS=1 Support (in-memory) 📋 **PLANNED**
**Target**: QoS=1 message delivery with in-memory persistence

**Features to Implement:**
- 📋 QoS=1 (At least once) message delivery
- 📋 Message acknowledgment (PUBACK) handling
- 📋 Message retransmission logic with exponential backoff
- 📋 Packet identifier management
- 📋 In-memory message persistence for unacknowledged messages
- 📋 Duplicate message detection and handling
- 📋 Session state management for QoS=1

**Technical Challenges:**
- Message deduplication algorithms
- Retry logic with exponential backoff
- In-memory storage for unacknowledged messages
- Session state recovery on reconnection
- Performance under high QoS=1 load

**Timeline**: 6-8 weeks
**Success Criteria**:
- [ ] QoS=1 messages delivered at least once
- [ ] Proper handling of duplicate messages
- [ ] Message retransmission on timeout
- [ ] Performance maintained under load

---

### Milestone 3: Basic Persistency & QoS=2 💾 **PLANNED**
**Target**: Persistent storage interface and QoS=2 support

**Features to Implement:**
- 💾 Persistent storage interface with pluggable backends
- 💾 QoS=2 (Exactly once) message delivery
- 💾 Two-phase commit protocol (PUBREC/PUBREL/PUBCOMP)
- 💾 Message durability guarantees
- 💾 Session state persistence
- 💾 Retained message persistence
- 💾 File-based and SQLite storage implementations

**Technical Challenges:**
- Complex state machine for QoS=2 flow
- Transaction-like message handling
- Storage consistency guarantees
- Performance optimization for persistent storage

**Timeline**: 8-10 weeks
**Success Criteria**:
- [ ] QoS=2 messages delivered exactly once
- [ ] Data survives server restarts
- [ ] Storage backend configurable
- [ ] Acceptable performance with persistence

---

### Milestone 4: Basic Authentication 🔐 **PLANNED**
**Target**: Config file-based authentication

**Features to Implement:**
- 🔐 Username/password authentication
- 🔐 Config file-based user management
- 🔐 TLS/SSL support for TCP connections
- 🔐 Basic client certificate authentication
- 🔐 Authentication result caching
- 🔐 Secure configuration management

**Authentication Methods:**
- Built-in user database (config file)
- File-based authentication
- Client certificate authentication
- Future: Database auth, LDAP, OAuth2

**Timeline**: 6-8 weeks
**Success Criteria**:
- [ ] Only authenticated clients can connect
- [ ] Multiple authentication methods supported
- [ ] TLS encryption working
- [ ] Secure configuration practices

---

### Milestone 5: Enhanced Transport Layer 🔗 **PLANNED**
**Target**: TLS and multiple transport protocols

**Features to Implement:**
- 🔗 Enhanced TLS/SSL support
- 🔗 WebSocket MQTT support (ws://)
- 🔗 Secure WebSocket support (wss://)
- 🔗 Unix domain socket support
- 🔗 Protocol negotiation and detection
- 🔗 Cross-protocol message routing

**Supported Protocols:**
- `tcp://` - Plain TCP (existing)
- `tcp+tls://` - TLS-encrypted TCP
- `ws://` - WebSocket over HTTP
- `wss://` - WebSocket over HTTPS
- `unix://` - Unix domain sockets

**Timeline**: 8-10 weeks
**Success Criteria**:
- [ ] Multiple protocols working simultaneously
- [ ] Seamless message routing across protocols
- [ ] Web browser clients supported
- [ ] Performance maintained across protocols

---

### Milestone 6: Pluggable Architecture 🔧 **PLANNED**
**Target**: Pluggable persistence, authentication, and authorization

**Features to Implement:**
- 🔧 Pluggable authentication backends
- 🔧 Pluggable authorization providers
- 🔧 Pluggable persistence backends
- 🔧 Topic-based access control lists (ACLs)
- 🔧 Role-based access control (RBAC)
- 🔧 Dynamic configuration updates

**Plugin Types:**
- Authentication: Database, LDAP, OAuth2, JWT
- Authorization: File-based, database, external APIs
- Storage: PostgreSQL, MySQL, Redis, MongoDB
- Metrics: Prometheus, InfluxDB, custom collectors

**Timeline**: 10-12 weeks
**Success Criteria**:
- [ ] Plugin system architecture working
- [ ] Multiple backend implementations
- [ ] Runtime plugin loading
- [ ] Configuration-driven plugin selection

---

### Milestone 7: Production Ready 🏢 **PLANNED**
**Target**: Enhanced logging, documentation, and production features

**Features to Implement:**
- 🏢 Enhanced logging and structured metrics
- 🏢 Comprehensive documentation and examples
- 🏢 Performance optimization and tuning
- 🏢 Health checks and monitoring endpoints
- 🏢 Deployment guides and best practices
- 🏢 Single-node production deployment

**Production Features:**
- Prometheus metrics integration
- Structured logging with JSON output
- Health check endpoints
- Configuration validation
- Performance benchmarking
- Docker and Kubernetes deployment

**Timeline**: 8-10 weeks
**Success Criteria**:
- [ ] Production-ready single-node deployment
- [ ] Comprehensive monitoring and alerting
- [ ] Complete documentation
- [ ] Performance benchmarks validated

---

## Version Milestones

### v0.1.0 - MQTTv3 Foundation (Milestone 1) 🔄
- Complete MQTT v3.1.1 server
- QoS 0 messaging with routing
- Clean session, retained messages, will messages
- Keep-alive mechanism
- Event-driven architecture

### v0.2.0 - Quality of Service (Milestone 2) 📋
- QoS 1 support with acknowledgments
- In-memory message persistence
- Message retransmission logic
- Duplicate detection

### v0.3.0 - Persistence & QoS 2 (Milestone 3) 💾
- Pluggable storage interface
- QoS 2 exactly-once delivery
- Session state persistence
- File and SQLite backends

### v0.4.0 - Authentication (Milestone 4) 🔐
- Username/password authentication
- TLS/SSL support
- Client certificate authentication
- Config-based user management

### v0.5.0 - Multi-Protocol (Milestone 5) 🔗
- WebSocket support
- Multiple transport protocols
- Cross-protocol routing
- Browser client support

### v0.6.0 - Pluggable Architecture (Milestone 6) 🔧
- Plugin system
- Multiple auth/storage backends
- Topic-based authorization
- Dynamic configuration

### v1.0.0 - Production Ready (Milestone 7) 🏢
- Enhanced monitoring
- Complete documentation
- Performance optimization
- Production deployment ready

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

This roadmap provides a clear path toward building a production-ready, enterprise-grade MQTT broker while maintaining stability and performance at each milestone. The progressive approach ensures each milestone builds solid foundations for the next.