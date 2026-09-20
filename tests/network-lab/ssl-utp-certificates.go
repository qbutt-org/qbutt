// Generated only for isolated SSL-uTP fixtures. Never use these keys in production.
package main

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"math/big"
	"os"
	"path/filepath"
	"time"
)

func main() {
	if len(os.Args) != 2 {
		panic("expected output directory")
	}
	root := os.Args[1]
	if err := os.MkdirAll(root, 0700); err != nil {
		panic(err)
	}
	caKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		panic(err)
	}
	now := time.Now()
	ca := &x509.Certificate{SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: "qbutt-ssl-utp-fixture-CA"},
		NotBefore: now.Add(-time.Hour), NotAfter: now.Add(time.Hour), IsCA: true, BasicConstraintsValid: true,
		KeyUsage: x509.KeyUsageCertSign | x509.KeyUsageDigitalSignature}
	caDER, err := x509.CreateCertificate(rand.Reader, ca, ca, &caKey.PublicKey, caKey)
	if err != nil {
		panic(err)
	}
	peerKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		panic(err)
	}
	peer := &x509.Certificate{SerialNumber: big.NewInt(2), Subject: pkix.Name{CommonName: "ssl-utp.bin"},
		DNSNames: []string{"ssl-utp.bin"}, NotBefore: ca.NotBefore, NotAfter: ca.NotAfter,
		KeyUsage:    x509.KeyUsageDigitalSignature,
		ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth, x509.ExtKeyUsageServerAuth}}
	peerDER, err := x509.CreateCertificate(rand.Reader, peer, ca, &peerKey.PublicKey, caKey)
	if err != nil {
		panic(err)
	}
	keyDER, err := x509.MarshalPKCS8PrivateKey(peerKey)
	if err != nil {
		panic(err)
	}
	for name, block := range map[string]*pem.Block{
		"ca.pem":       {Type: "CERTIFICATE", Bytes: caDER},
		"peer.pem":     {Type: "CERTIFICATE", Bytes: peerDER},
		"peer-key.pem": {Type: "PRIVATE KEY", Bytes: keyDER},
	} {
		if err := os.WriteFile(filepath.Join(root, name), pem.EncodeToMemory(block), 0600); err != nil {
			panic(err)
		}
	}
	fingerprint := sha256.Sum256(peerDER)
	if err := json.NewEncoder(os.Stdout).Encode(map[string]string{"fingerprint": hex.EncodeToString(fingerprint[:])}); err != nil {
		panic(err)
	}
}
