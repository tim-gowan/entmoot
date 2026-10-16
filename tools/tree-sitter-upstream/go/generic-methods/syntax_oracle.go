// This is a syntax oracle only. It does not type-check or execute corpus code.
package main

import (
	"encoding/hex"
	"encoding/json"
	"go/parser"
	"go/token"
	"io"
	"os"
)

func main() {
	decoder, encoder := json.NewDecoder(os.Stdin), json.NewEncoder(os.Stdout)
	for {
		var request struct{ Path, SourceHex string }
		if err := decoder.Decode(&request); err == io.EOF {
			return
		} else if err != nil {
			panic(err)
		}
		source, err := hex.DecodeString(request.SourceHex)
		if err != nil {
			panic(err)
		}
		_, err = parser.ParseFile(token.NewFileSet(), request.Path, source, parser.AllErrors|parser.ParseComments)
		result := struct {
			Accepted bool   `json:"accepted"`
			Reason   string `json:"reason,omitempty"`
		}{Accepted: err == nil}
		if err != nil {
			result.Reason = err.Error()
		}
		if err := encoder.Encode(result); err != nil {
			panic(err)
		}
	}
}
