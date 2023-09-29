package main

import (
	"encoding/json"
	//"crypto/ecdsa"
	//"crypto/elliptic"
	//"crypto/rand"
	//"fmt"
	"fmt"
	"runtime/cgo"
	"strings"
	"sync"
	"unsafe"

	"github.com/pion/webrtc/v3"
)

type PeerCon struct {
	mu     sync.Mutex
	closed bool
	con    *webrtc.PeerConnection
	handle UintPtrT
}

func (peerCon *PeerCon) Free() {
	peerCon.mu.Lock()
	defer peerCon.mu.Unlock()

	if peerCon.closed {
		return
	}

	peerCon.closed = true
	(cgo.Handle)(peerCon.handle).Delete()

	peerCon.con.Close()
	peerCon.con = nil
}

type PeerConConfig struct {
	ICEServers   []webrtc.ICEServer `json:"iceServers,omitempty"`
	Certificates []string           `json:"certificates,omitempty"`
}

func CallPeerConAlloc(
	config_buf_id UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	/*
	  sk, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	  if err != nil {
	    panic(err)
	  }
	  certificate, err := webrtc.GenerateCertificate(sk)
	  if err != nil {
	    panic(err)
	  }
	  certPem, err := certificate.PEM()
	  if err != nil {
	    panic(err)
	  }
	  fmt.Printf("cert:\n%s\n", certPem)
	*/
	buf := BufferFromPtr(config_buf_id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	var tmpConfig PeerConConfig
	if err := json.Unmarshal(buf.buf.Bytes(), &tmpConfig); err != nil {
		errStr := fmt.Sprintf("%s: %s", err, buf.buf.Bytes())
		panic(errStr)
	}

	var config_parsed webrtc.Configuration
	config_parsed.ICEServers = tmpConfig.ICEServers

	for _, certPem := range tmpConfig.Certificates {
		cert, err := webrtc.CertificateFromPEM(certPem)
		if err != nil {
			panic(err)
		}
		config_parsed.Certificates = append(config_parsed.Certificates, *cert)
	}

	webrtcApi := getWebrtcApi()

	con, err := webrtcApi.NewPeerConnection(config_parsed)
	if err != nil {
		panic(err)
	}

	peerCon := new(PeerCon)
	peerCon.con = con

	handle := UintPtrT(cgo.NewHandle(peerCon))
	peerCon.handle = handle

	EmitTrace(
		LvlDebug,
		fmt.Sprintf(
			"PeerConnection(%v).PeerConAlloc()",
			handle,
		),
	)

	con.OnICECandidate(func(candidate *webrtc.ICECandidate) {
		EmitTrace(
			LvlTrace,
			fmt.Sprintf(
				"PeerConnection(%v).OnICECandidate(%v)",
				handle,
				candidate,
			),
		)

		if candidate == nil {
			return
		}

		json, err := json.Marshal(candidate.ToJSON())
		if err != nil {
			return
		}

		buf := NewBuffer([]byte(json))

		EmitEvent(
			TyPeerConOnICECandidate,
			handle,
			buf.handle,
			0,
			0,
		)
	})

	con.OnConnectionStateChange(func(state webrtc.PeerConnectionState) {
		EmitEvent(
			TyPeerConOnStateChange,
			handle,
			UintPtrT(state),
			0,
			0,
		)
	})

	con.OnDataChannel(func(ch *webrtc.DataChannel) {
		dataChan := NewDataChan(ch)

		EmitTrace(
			LvlTrace,
			fmt.Sprintf(
				"PeerConnection(%v).OnDataChannel(%v)",
				handle,
				dataChan.handle,
			),
		)

		EmitEvent(
			TyPeerConOnDataChannel,
			handle,
			dataChan.handle,
			0,
			0,
		)
	})

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyPeerConAlloc,
		peerCon.handle,
		0,
		0,
		0,
	)
}

func CallPeerConFree(peer_con_id UintPtrT) {
	hnd := cgo.Handle(peer_con_id)
	peerCon := hnd.Value().(*PeerCon)
	peerCon.Free()
}

func CallPeerConStats(
	peer_con_id UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	hnd := cgo.Handle(peer_con_id)
	peerCon := hnd.Value().(*PeerCon)
	peerCon.mu.Lock()
	defer peerCon.mu.Unlock()

	if peerCon.closed {
		panic("PeerConClosed")
	}

	stats := peerCon.con.GetStats()

	for k, _ := range stats {
		if strings.HasPrefix(k, "candidate") || strings.HasPrefix(k, "certificate") {
			delete(stats, k)
		}
	}

	json, err := json.Marshal(stats)
	if err != nil {
		panic(err)
	}

	bufOut := NewBuffer([]byte(json))

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyPeerConStats,
		bufOut.handle,
		0,
		0,
		0,
	)
}

func CallPeerConCreateOffer(
	peer_con_id UintPtrT,
	config_buf_id UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	hnd := cgo.Handle(peer_con_id)
	peerCon := hnd.Value().(*PeerCon)
	peerCon.mu.Lock()
	defer peerCon.mu.Unlock()

	if peerCon.closed {
		panic("PeerConClosed")
	}

	buf := BufferFromPtr(config_buf_id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	opts := new(webrtc.OfferOptions)
	if err := json.Unmarshal(buf.buf.Bytes(), opts); err != nil {
		panic(err)
	}

	offer, err := peerCon.con.CreateOffer(opts)
	if err != nil {
		panic(err)
	}

	offerJson, err := json.Marshal(offer)
	if err != nil {
		panic(err)
	}

	bufOut := NewBuffer([]byte(offerJson))

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyPeerConCreateOffer,
		bufOut.handle,
		0,
		0,
		0,
	)
}

func CallPeerConCreateAnswer(
	peer_con_id UintPtrT,
	config_buf_id UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	hnd := cgo.Handle(peer_con_id)
	peerCon := hnd.Value().(*PeerCon)
	peerCon.mu.Lock()
	defer peerCon.mu.Unlock()

	if peerCon.closed {
		panic("PeerConClosed")
	}

	buf := BufferFromPtr(config_buf_id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	opts := new(webrtc.AnswerOptions)
	if err := json.Unmarshal(buf.buf.Bytes(), opts); err != nil {
		panic(err)
	}

	offer, err := peerCon.con.CreateAnswer(opts)
	if err != nil {
		panic(err)
	}

	offerJson, err := json.Marshal(offer)
	if err != nil {
		panic(err)
	}

	bufOut := NewBuffer([]byte(offerJson))

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyPeerConCreateAnswer,
		bufOut.handle,
		0,
		0,
		0,
	)
}

func CallPeerConSetLocalDesc(
	peer_con_id UintPtrT,
	desc_buf_id UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	hnd := cgo.Handle(peer_con_id)
	peerCon := hnd.Value().(*PeerCon)
	peerCon.mu.Lock()
	defer peerCon.mu.Unlock()

	if peerCon.closed {
		panic("PeerConClosed")
	}

	buf := BufferFromPtr(desc_buf_id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	var desc webrtc.SessionDescription
	if err := json.Unmarshal(buf.buf.Bytes(), &desc); err != nil {
		panic(err)
	}

	if err := peerCon.con.SetLocalDescription(desc); err != nil {
		panic(err)
	}

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyPeerConSetLocalDesc,
		0,
		0,
		0,
		0,
	)
}

func CallPeerConSetRemDesc(
	peer_con_id UintPtrT,
	desc_buf_id UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	hnd := cgo.Handle(peer_con_id)
	peerCon := hnd.Value().(*PeerCon)
	peerCon.mu.Lock()
	defer peerCon.mu.Unlock()

	if peerCon.closed {
		panic("PeerConClosed")
	}

	buf := BufferFromPtr(desc_buf_id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	var desc webrtc.SessionDescription
	if err := json.Unmarshal(buf.buf.Bytes(), &desc); err != nil {
		panic(err)
	}

	if err := peerCon.con.SetRemoteDescription(desc); err != nil {
		panic(err)
	}

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyPeerConSetRemDesc,
		0,
		0,
		0,
		0,
	)
}

func CallPeerConAddICECandidate(
	peer_con_id UintPtrT,
	ice_buf_id UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	hnd := cgo.Handle(peer_con_id)
	peerCon := hnd.Value().(*PeerCon)
	peerCon.mu.Lock()
	defer peerCon.mu.Unlock()

	if peerCon.closed {
		panic("PeerConClosed")
	}

	buf := BufferFromPtr(ice_buf_id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	var candidate webrtc.ICECandidateInit
	if err := json.Unmarshal(buf.buf.Bytes(), &candidate); err != nil {
		panic(err)
	}

	if err := peerCon.con.AddICECandidate(candidate); err != nil {
		panic(err)
	}

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyPeerConAddICECandidate,
		0,
		0,
		0,
		0,
	)
}

type DataChanInit struct {
	Ordered           *bool   `json:"ordered,omitempty"`
	MaxPacketLifeTime *uint16 `json:"maxPacketLifeTime,omitempty"`
}

type DataChanConfig struct {
	Label string        `json:"label,omitempty"`
	Init  *DataChanInit `json:"init,omitempty"`
}

func CallPeerConCreateDataChan(
	peer_con_id UintPtrT,
	config_buf_id UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	hnd := cgo.Handle(peer_con_id)
	peerCon := hnd.Value().(*PeerCon)
	peerCon.mu.Lock()
	defer peerCon.mu.Unlock()

	if peerCon.closed {
		panic("PeerConClosed")
	}

	buf := BufferFromPtr(config_buf_id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	var conf DataChanConfig
	if err := json.Unmarshal(buf.buf.Bytes(), &conf); err != nil {
		panic(err)
	}

	var init *webrtc.DataChannelInit = nil

	if conf.Init != nil &&
		(conf.Init.Ordered != nil ||
			conf.Init.MaxPacketLifeTime != nil) {
		init = new(webrtc.DataChannelInit)
		init.Ordered = conf.Init.Ordered
		init.MaxPacketLifeTime = conf.Init.MaxPacketLifeTime
	}

	ch, err := peerCon.con.CreateDataChannel(conf.Label, init)
	if err != nil {
		panic(err)
	}

	dataChan := NewDataChan(ch)

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyPeerConCreateDataChan,
		dataChan.handle,
		0,
		0,
		0,
	)
}
