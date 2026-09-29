package user_test

import (
	"testing"

	"example.com/differential/user"
)

func TestFromOutside(t *testing.T) {
	if !user.Valid("x") {
		t.Fatal("invalid")
	}
}
