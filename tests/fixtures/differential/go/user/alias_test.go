package user

import tt "testing"

func TestThroughAnAlias(t *tt.T) {
	if !Valid("x") {
		t.Fatal("invalid")
	}
}
