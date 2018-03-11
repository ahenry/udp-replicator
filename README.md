This is the start of a UDP proxy/load balancer/duplicator.

You see, I have a bunch of netflow traffic that I want in different places, for different uses.
There are a couple of other things out there that do bits of what I want:
	Pen
	NGINX
	Others, I'm sure

But I'd like something that can load balance and duplicate, and have health checks for the
load-balanced endpoints.  I didn't find anything like that already written, so...
