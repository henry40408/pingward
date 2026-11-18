// Package ids generates random identifiers.
package ids

import (
	"crypto/rand"
	"fmt"
)

// NewV4 returns a random UUID in lowercase 8-4-4-4-12 form.
func NewV4() string {
	var u [16]byte
	if _, err := rand.Read(u[:]); err != nil {
		panic("ids: OS random source failed: " + err.Error())
	}
	u[6] = u[6]&0x0f | 0x40
	u[8] = u[8]&0x3f | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", u[0:4], u[4:6], u[6:8], u[8:10], u[10:16])
}

// NewV4Simple returns a random UUID without hyphens (32 hex characters).
func NewV4Simple() string {
	var u [16]byte
	if _, err := rand.Read(u[:]); err != nil {
		panic("ids: OS random source failed: " + err.Error())
	}
	u[6] = u[6]&0x0f | 0x40
	u[8] = u[8]&0x3f | 0x80
	return fmt.Sprintf("%x", u[:])
}
