package user

import (
	"fmt"
	"os"
	"testing"
)

func TestMain(m *testing.M) {
	os.Exit(m.Run())
}

func TestCreateUser(t *testing.T) {
	t.Run("with valid name", func(t *testing.T) {
		if !Valid("Alice") {
			t.Fatal("invalid")
		}
	})
}

func TestValidateEmail(t *testing.T) {
	cases := []struct {
		name  string
		email string
	}{
		{"standard", "a@example.com"},
		{"empty", ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) { _ = tc.email })
	}
}

func Test_underscore_name(t *testing.T) {}

func testHelper(t *testing.T) {}

func BenchmarkValid(b *testing.B) {
	for i := 0; i < b.N; i++ {
		Valid("x")
	}
}

func FuzzValid(f *testing.F) {
	f.Fuzz(func(t *testing.T, s string) { Valid(s) })
}

func ExampleValid() {
	fmt.Println(Valid("x"))
	// Output: true
}
