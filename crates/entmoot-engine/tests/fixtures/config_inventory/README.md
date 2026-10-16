# Configuration and runtime inventory fixtures

`defectdojo-rendered.yaml` is a small, sanitized rendering of the connector
shape in the upstream DefectDojo Helm chart at commit
`254dab83759d0d2fd17b5610d7d8413c790c608a`. It contains no secrets and uses
non-routable example image and host names.

The fixture deliberately represents **desired configuration**, not proof of a
deployment. The `defectdojo_config_mvp` integration test adds a separate
immutable observed-resource artifact. That artifact explicitly states the
desired-to-observed stitches and runtime connections; the parser never guesses
database or broker connections from environment-variable names.

Expected MVP shape:

- 12 declared Kubernetes resources;
- 12 syntax-native relations (Ingress→Service, Service→workload, and workload
  configuration references);
- 12 observed resource occurrences and 12 explicit `observed_as` stitches;
- 8 explicit process→PostgreSQL/Valkey runtime connector relations;
- one Checkov-shaped SARIF finding attached through the generic SARIF adapter to
  the enclosing Deployment resource.

The source Helm templates contain Go-template expressions and therefore must be
rendered into an immutable manifest before configuration inventory. This is a
soundness boundary, not a parser limitation to bypass heuristically.
