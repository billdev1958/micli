//! Ejecución aislada en Docker vía `bollard`, con imagen parametrizada por
//! proyecto.
//!
//! Fase 5: sin red (o allowlist), límites de CPU/RAM/tiempo, filesystem
//! read-only salvo el workspace. Nunca montar credenciales, variables de
//! entorno del host ni el socket de Docker dentro del contenedor.
