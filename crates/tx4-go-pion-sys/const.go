package main

const (
	// Reporting an error from the lib.
	// - allowed contexts: Event, Response
	// - msg slot_a = utf8 error id ptr
	// - msg slot_b = utf8 error id ptr
	// - msg slot_c = utf8 error info ptr
	// - msg slot_d = utf8 error info len
	TyErr UintPtrT = 0xffff

	// Request a go buffer be created / giving access to said buffer in resp.
	// - allowed contexts: Call, Response
	// - msg slot_a: buffer id
	TyBufferAlloc UintPtrT = 0x8001

	// Request an existing buffer be released. It will no longer be accessible.
	// - allowed contexts: Call, Response
	// - call slot_a: buffer id
	TyBufferFree UintPtrT = 0x8002

	// Request access to an existing buffer.
	// - allowed contexts: Call, Response
	// - call slot_a: buffer id
	// - msg slot_a: buffer id
	// - msg slot_b: buffer ptr
	// - msg slot_c: buffer len
	TyBufferAccess UintPtrT = 0x8003

	// Request additional space be reserved for appending to buffer.
	// - allowed contexts: Call, Response
	// - call slot_a: buffer id
	// - call slot_b: additional length
	TyBufferReserve UintPtrT = 0x8004

	// Request existing buffer be extended with provided additional bytes.
	// - allowed contexts: Call, Response
	// - call slot_a: buffer id
	// - call slot_b: additional bytes ptr
	// - call slot_c: additional bytes len
	TyBufferExtend UintPtrT = 0x8005

	// Request access to bytes at beginning of existing buffer, advancing the
	// read cursor.
	// - allowed contexts: Call, Response
	// - call slot_a: buffer id
	// - call slot_b: max read length
	// - msg slot_a: buffer ptr
	// - msg slot_b: buffer len
	TyBufferRead UintPtrT = 0x8006

	// Request a new peer connection be opened.
	// - allowed contexts: Call, Response
	// - call slot_a: utf8 json config ptr
	// - call slot_b: utf8 json config len
	// - msg slot_a: peer_con id
	TyPeerConAlloc UintPtrT = 0x9001

	// Request an existing peer con be closed and released.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	TyPeerConFree UintPtrT = 0x9002

	// Request an existing peer con create an offer.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: utf8 json ptr
	// - call slot_c: utf8 json len
	// - msg slot_a: utf8 json ptr
	// - msg slot_b: utf8 json len
	TyPeerConCreateOffer UintPtrT = 0x9003

	// Request an existing peer con create an answer.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: utf8 json ptr
	// - call slot_c: utf8 json len
	// - msg slot_a: utf8 json ptr
	// - msg slot_b: utf8 json len
	TyPeerConCreateAnswer UintPtrT = 0x9004

	// Request an existing peer con set local description.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: utf8 json ptr
	// - call slot_c: utf8 json len
	TyPeerConSetLocalDesc UintPtrT = 0x9005

	// Request an existing peer con set rem description.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: utf8 json ptr
	// - call slot_c: utf8 json len
	TyPeerConSetRemDesc UintPtrT = 0x9006

	// Request an existing peer con add ice candidate.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: utf8 json ptr
	// - call slot_c: utf8 json len
	TyPeerConAddICECandidate UintPtrT = 0x9007

	// Request an existing peer con create a new data channel.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: utf8 json ptr
	// - call slot_c: utf8 json len
	TyPeerConCreateDataChan UintPtrT = 0x9008

	// Request the remote certificate (if available).
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: cert ptr
	// - call slot_c: cert len
	TyPeerConRemCert UintPtrT = 0x9009

	// OnICECandidate event on an existing peer con.
	// - allowed contexts: Event
	// - msg slot_a: peer_con id
	// - msg slot_b: utf8 ptr
	// - msg slot_c: utf8 len
	TyPeerConOnICECandidate UintPtrT = 0x9801

	// OnConStateChange event on an existing peer con.
	// - allowed contexts: Event
	// - msg slot_a: peer_con id
	// - msg slot_b: state id
	TyPeerConOnStateChange UintPtrT = 0x9802

	// OnDataChannel event on an existing peer con.
	// - allowed contexts: Event
	// - msg slot_a: peer_con id
	// - msg slot_b: data_chan id
	TyPeerConOnDataChannel UintPtrT = 0x9803

	// Request an existing data chan be closed and released.
	// - allowed contexts: Call, Response
	// - call slot_a: data_chan id
	TyDataChanFree UintPtrT = 0xa002

	// Request the ready state of an existing data channel..
	// - allowed contexts: Call, Response
	// - call slot_a: data_chan id
	// - msg slot_a: ready state
	TyDataChanReadyState UintPtrT = 0xa003

	// Request an existing data chan send data to the remote end.
	// - allowed contexts: Call, Response
	// - call slot_a: data_chan id
	// - call slot_b: buffer id
	TyDataChanSend UintPtrT = 0xa004

	// OnClose event on an existing data chan.
	// - allowed contexts: Event
	// - msg slot_a: data_chan id
	TyDataChanOnClose UintPtrT = 0xa801

	// OnOpen event on an existing data chan.
	// - allowed contexts: Event
	// - msg slot_a: data_chan id
	TyDataChanOnOpen UintPtrT = 0xa802

	// OnError event on an existing data chan.
	// - allowed contexts: Event
	// - msg slot_a: data_chan id
	// - msg slot_b: data ptr
	// - msg slot_c: data len
	TyDataChanOnError UintPtrT = 0xa803

	// OnMessage event on an existing data chan.
	// - allowed contexts: Event
	// - msg slot_a: data_chan id
	// - msg slot_b: data ptr
	// - msg slot_c: data len
	TyDataChanOnMessage UintPtrT = 0xa804
)
