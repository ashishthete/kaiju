# CLAUDE.md

## Purpose

This project prioritizes:

1. Simplicity (KISS)
2. Single Responsibility Principle (SRP)
3. Reusability
4. Maintainability
5. Explicitness over magic
6. Testability
7. Performance only when measured

When multiple solutions are possible:

1. Choose the simplest correct solution.
2. Prefer readability over cleverness.
3. Prefer composition over inheritance.
4. Avoid premature optimization.
5. Minimize dependencies.

---

# Core Engineering Principles

## KISS (Keep It Simple)

Always prefer:

* Straightforward control flow
* Small functions
* Explicit types
* Predictable behavior

Avoid:

* Clever abstractions
* Deep generic hierarchies
* Over-engineering
* Excessive macro usage

Bad:

```rust
let result = data
    .iter()
    .flat_map(...)
    .filter(...)
    .fold(...);
```

Good:

```rust
for item in data {
    if should_process(item) {
        process(item);
    }
}
```

Readability wins.

---

## Single Responsibility Principle

Every:

* Function
* Struct
* Module
* Crate

must have one reason to change.

Bad:

```rust
struct UserService {
    fn validate() {}
    fn save_to_db() {}
    fn send_email() {}
}
```

Good:

```rust
ValidationService
UserRepository
NotificationService
```

Separate concerns.

---

## Composition Over Inheritance

Prefer:

```rust
struct OrderService {
    repository: OrderRepository,
    notifier: NotificationService,
}
```

Avoid large inheritance-like hierarchies.

---

## Dependency Rule

Business logic must never depend on:

* Database implementation
* Web framework
* External APIs
* UI

Instead depend on traits.

Example:

```rust
trait UserRepository {
    fn find_by_id(&self, id: UserId) -> Result<User>;
}
```

Inject implementations.

---

# Project Structure

Preferred:

```text
src/
├── domain/
├── application/
├── infrastructure/
├── interfaces/
├── shared/
└── main.rs
```

## Domain

Contains:

* Entities
* Value Objects
* Domain Rules

Must not depend on:

* Axum
* Tokio
* SQLx
* Diesel
* External services

---

## Application

Contains:

* Use cases
* Commands
* Queries
* Orchestration

May depend on domain.

---

## Infrastructure

Contains:

* Database
* HTTP clients
* File systems
* External integrations

Implements traits defined elsewhere.

---

## Interfaces

Contains:

* REST APIs
* CLI
* gRPC
* Message consumers

Thin layer only.

---

# Function Design

Functions should:

* Do one thing
* Be easy to test
* Have explicit names

Prefer:

```rust
fn calculate_total()
```

over:

```rust
fn process()
```

Target:

* < 30 lines preferred
* < 50 lines maximum

Refactor when exceeded.

---

# Error Handling

Never:

```rust
unwrap()
expect()
panic!()
```

except:

* tests
* startup validation
* truly unrecoverable states

Prefer:

```rust
Result<T, AppError>
```

Use:

```rust
thiserror
```

for custom errors.

---

# Traits

Use traits only when:

* Multiple implementations exist
* Dependency inversion is needed
* Testing requires mocking

Do not create traits with a single implementation without a clear reason.

---

# Design Patterns

Use patterns sparingly.

Preferred:

## Repository Pattern

```rust
trait UserRepository
```

---

## Strategy Pattern

```rust
trait PaymentProcessor
```

---

## Factory Pattern

Only when construction becomes complex.

---

## Builder Pattern

For complex configuration objects.

---

Avoid:

* God Objects
* Service Locator
* Deep Factory Chains
* Abstract Factory unless justified

---

# State Management

Prefer immutable data.

Use mutable state only when necessary.

Keep mutation localized.

---

# Async Guidelines

Use async only when:

* I/O bound
* Network calls
* Database access

Do not make CPU-bound work async.

Avoid nested async chains.

---

# Testing

Required:

## Unit Tests

Test:

* Domain logic
* Business rules
* Edge cases

## Integration Tests

Test:

* Database interactions
* API contracts
* External adapters

Target:

* Meaningful coverage
* Not coverage percentage

---

# Logging

Use structured logs.

Include:

* request_id
* correlation_id
* entity_id

Never log:

* passwords
* tokens
* secrets
* PII

---

# Security

Validate all external input.

Use:

* Least privilege
* Secure defaults
* Explicit authorization

Never trust client input.

---

# Code Review Checklist

Before submitting:

* Is this the simplest solution?
* Can any abstraction be removed?
* Does every module have one responsibility?
* Can code be reused?
* Are errors handled correctly?
* Are tests included?
* Are dependencies necessary?
* Is naming clear?
* Would a new developer understand this in 5 minutes?

If not, refactor.

---

# AI Agent Instructions

When generating code:

1. Prefer simplicity over sophistication.
2. Follow SRP strictly.
3. Avoid unnecessary abstractions.
4. Reuse existing modules before creating new ones.
5. Minimize dependencies.
6. Generate tests with implementation.
7. Explain architectural trade-offs.
8. Never introduce patterns without justification.
9. Favor composition.
10. Produce production-ready Rust code.

## Reuse First

Before creating:

- new service
- new crate
- new component
- new API

Search existing codebase for reusable functionality.

Prefer extending existing modules over creating duplicates.