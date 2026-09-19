// Generated only for isolated gateway process fixtures. Never use these keys in production.
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
	"net"
	"os"
	"path/filepath"
	"time"
)

type pair struct{ cert, der, key []byte }

func issue(ca *x509.Certificate, caKey *ecdsa.PrivateKey, server bool, serverIP net.IP) pair {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		panic(err)
	}
	serial, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 120))
	if err != nil {
		panic(err)
	}
	template := &x509.Certificate{SerialNumber: serial, Subject: pkix.Name{CommonName: "qbutt-generated-gateway-fixture"},
		NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(time.Hour), KeyUsage: x509.KeyUsageDigitalSignature,
		ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth}}
	if server {
		template.ExtKeyUsage = []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}
		template.IPAddresses = []net.IP{serverIP}
	}
	der, err := x509.CreateCertificate(rand.Reader, template, ca, &key.PublicKey, caKey)
	if err != nil {
		panic(err)
	}
	encoded, err := x509.MarshalPKCS8PrivateKey(key)
	if err != nil {
		panic(err)
	}
	return pair{pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), der,
		pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: encoded})}
}

func write(path string, value []byte) {
	if err := os.WriteFile(path, value, 0600); err != nil {
		panic(err)
	}
}

func main() {
	if len(os.Args) != 3 {
		panic("expected output directory and numeric server IP")
	}
	serverIP := net.ParseIP(os.Args[2])
	if serverIP == nil {
		panic("numeric server IP required")
	}
	root := os.Args[1]
	if err := os.MkdirAll(root, 0700); err != nil {
		panic(err)
	}
	caKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		panic(err)
	}
	ca := &x509.Certificate{SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: "qbutt-gateway-fixture-CA"},
		NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(time.Hour), IsCA: true, BasicConstraintsValid: true,
		KeyUsage: x509.KeyUsageCertSign | x509.KeyUsageDigitalSignature}
	caDER, err := x509.CreateCertificate(rand.Reader, ca, ca, &caKey.PublicKey, caKey)
	if err != nil {
		panic(err)
	}
	server := issue(ca, caKey, true, serverIP)
	client := issue(ca, caKey, false, serverIP)
	write(filepath.Join(root, "ca.pem"), pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: caDER}))
	write(filepath.Join(root, "server.pem"), server.cert)
	write(filepath.Join(root, "server-key.pem"), server.key)
	write(filepath.Join(root, "client.pem"), client.cert)
	write(filepath.Join(root, "client-key.pem"), client.key)
	fingerprint := sha256.Sum256(client.der)
	json.NewEncoder(os.Stdout).Encode(map[string]string{"fingerprint": hex.EncodeToString(fingerprint[:])})
}
