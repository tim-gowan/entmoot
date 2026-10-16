// Windows/Linux helper binary used as ENTMOOT_GO_ADAPTER when Docker provides Go.
// Reads the sidecar JSON request on stdin, rewrites root → /src, runs the Linux
// image, rewrites /src paths back to the host root, writes JSON on stdout.
package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

const containerRoot = "/src"

func main() {
	var request map[string]any
	if err := json.NewDecoder(os.Stdin).Decode(&request); err != nil {
		fail(fmt.Sprintf("invalid request JSON: %v", err))
	}
	root, _ := request["root"].(string)
	root = strings.TrimSpace(root)
	if root == "" {
		fail("request.root is required")
	}
	hostRoot, err := filepath.Abs(root)
	if err != nil {
		fail(err.Error())
	}
	hostRoot = strings.TrimPrefix(hostRoot, `\\?\`)
	hostRootSlash := filepath.ToSlash(hostRoot)
	if st, err := os.Stat(hostRoot); err != nil || !st.IsDir() {
		fail(fmt.Sprintf("root is not a directory: %s", hostRoot))
	}

	image := envOr("ENTMOOT_GO_DOCKER_IMAGE", "entmoot-go-sidecar:1.26")
	modVolume := envOr("ENTMOOT_GO_DOCKER_MODCACHE_VOLUME", "entmoot-go-modcache")
	here, err := filepath.Abs(filepath.Dir(os.Args[0]))
	if err != nil {
		here = "."
	}
	if err := ensureImage(image, here); err != nil {
		respondInvalid(err.Error())
		return
	}

	request["root"] = containerRoot
	payload, err := json.Marshal(request)
	if err != nil {
		fail(err.Error())
	}

	volume := hostRootSlash + ":" + containerRoot + ":ro"
	// Forward host sidecar env the engine sets (especially ENTMOOT_GO_SINGLE_MODULE).
	args := []string{
		"run", "--rm", "-i",
		"-v", volume,
		"-v", modVolume + ":/go/pkg/mod",
		"-e", "ENTMOOT_GO_GOPROXY=off",
		"-e", "ENTMOOT_GO_GOTOOLCHAIN=local",
		"-e", "ENTMOOT_GO_PREWARM_GOPROXY=" + envOr("ENTMOOT_GO_PREWARM_GOPROXY", "https://proxy.golang.org,direct"),
	}
	args = append(args, sidecarEnvironment()...)
	args = append(args, image)
	cmd := exec.Command("docker", args...)
	cmd.Stdin = bytes.NewReader(payload)
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	if err := cmd.Run(); err != nil {
		respondInvalid(fmt.Sprintf("docker run failed: %v; stderr=%s", err, strings.TrimSpace(stderr.String())))
		return
	}
	var response any
	if err := json.Unmarshal(stdout.Bytes(), &response); err != nil {
		respondInvalid(fmt.Sprintf("sidecar returned invalid JSON: %v; stderr=%s", err, strings.TrimSpace(stderr.String())))
		return
	}
	rewritten := rewritePaths(response, hostRootSlash)
	enc := json.NewEncoder(os.Stdout)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(rewritten); err != nil {
		fail(err.Error())
	}
}

func sidecarEnvironment() []string {
	var args []string
	for _, key := range []string{"ENTMOOT_GO_SINGLE_MODULE", "ENTMOOT_GO_TAGS", "ENTMOOT_GO_TESTS"} {
		if value := strings.TrimSpace(os.Getenv(key)); value != "" {
			args = append(args, "-e", key+"="+value)
		}
	}
	return args
}

func ensureImage(image, here string) error {
	inspect := exec.Command("docker", "image", "inspect", image)
	if inspect.Run() == nil {
		return nil
	}
	compose := filepath.Join(here, "docker-compose.yml")
	if _, err := os.Stat(compose); err != nil {
		compose = filepath.Join(here, "..", "docker-compose.yml")
	}
	build := exec.Command("docker", "compose", "-f", compose, "build", "go-sidecar")
	build.Dir = filepath.Dir(compose)
	out, err := build.CombinedOutput()
	if err != nil {
		return fmt.Errorf("docker compose build failed: %s", strings.TrimSpace(string(out)))
	}
	if exec.Command("docker", "image", "inspect", image).Run() != nil {
		return fmt.Errorf("image %s missing after compose build", image)
	}
	return nil
}

func rewritePaths(value any, hostRoot string) any {
	switch typed := value.(type) {
	case string:
		normalized := filepath.ToSlash(typed)
		if normalized == containerRoot || strings.HasPrefix(normalized, containerRoot+"/") {
			suffix := strings.TrimPrefix(normalized[len(containerRoot):], "/")
			if suffix == "" {
				return filepath.FromSlash(hostRoot)
			}
			return filepath.FromSlash(hostRoot + "/" + suffix)
		}
		return typed
	case []any:
		out := make([]any, len(typed))
		for i, item := range typed {
			out[i] = rewritePaths(item, hostRoot)
		}
		return out
	case map[string]any:
		out := make(map[string]any, len(typed))
		for key, item := range typed {
			out[key] = rewritePaths(item, hostRoot)
		}
		return out
	default:
		return value
	}
}

func respondInvalid(message string) {
	_ = json.NewEncoder(os.Stdout).Encode(map[string]any{
		"build_context":       map[string]any{},
		"compiler_version":    "docker_sidecar_proxy",
		"calls":               []any{},
		"inputs":              []any{},
		"diagnostics":         []string{message},
		"invalid_environment": true,
	})
}

func fail(message string) {
	fmt.Fprintln(os.Stderr, "entmoot-go-resolver-docker:", message)
	os.Exit(2)
}

func envOr(key, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(key)); value != "" {
		return value
	}
	return fallback
}
